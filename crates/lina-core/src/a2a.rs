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

/// Intervalo de polling do grid no `wait_ready`. O ORÇAMENTO da espera vem do perfil
/// (`CliProfile::ready_timeout`, default 2s) — F1-0-2: timeout → **não injeta** (o
/// "injeta mesmo assim" histórico era o mecanismo do texto-colado 18/20 da baseline).
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
    // ACHADO-2: o antigo `NotReady` (prompt não-pronto = FALHA) FOI REMOVIDO. Prompt-não-pronto
    // deixou de ser erro — virou sinal de OCUPADO, devolvido como `Ok(DeliveryOutcome::Injected
    // { ready: false })` para o router RETER (não strike/DLQ). Erros aqui são só falhas REAIS
    // (regex inválida, allow-list, supervisor — PTY morto/lock/alvo inexistente).
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
    /// F1-0-2: texto da REGIÃO DE INPUT do grid (da última linha-prompt ao fim; ver a
    /// impl real). É o que o `prompt_ready_regex` multiline e os `busy_markers` julgam —
    /// a ÚLTIMA linha sozinha é o sensor errado (na TUI do Claude Code ela é o rodapé
    /// de atalhos, nunca o prompt — grids reais da baseline F1-0). Default compatível
    /// para sensores simples: a última linha não-vazia.
    fn input_region_text(&self) -> String {
        self.last_nonempty_line()
    }
}

/// Linha de prompt vivo da TUI: `>` ou `❯` no início seguido de whitespace (espaço
/// comum na v2.1.166 — grids da baseline; **NBSP** U+00A0 na v2.1.167 — prova com
/// claude vivo do gate F1-0-2), opcionalmente atrás da borda `│`. Sem aceitar o NBSP,
/// a âncora da região cai numa linha de ECO do histórico (`❯ msg já submetida`) e o
/// sensor erra a região inteira — a deriva de TUI que a baseline §3 previu.
fn is_prompt_row(row: &str) -> bool {
    let mut t = row.trim_start();
    if let Some(rest) = t.strip_prefix('│') {
        t = rest.trim_start();
    }
    for glyph in ['>', '❯'] {
        if let Some(rest) = t.strip_prefix(glyph) {
            // fim de linha ("❯") ou glifo seguido de QUALQUER whitespace unicode
            // (espaço, NBSP…) — mesma família que o `\s` do prompt_ready_regex casa.
            if rest.is_empty() || rest.chars().next().is_some_and(char::is_whitespace) {
                return true;
            }
        }
    }
    false
}

/// Fallback quando nenhuma linha-prompt existe no viewport: últimas N linhas não-vazias
/// (mesma constante da calibração da baseline).
const REGION_FALLBACK_ROWS: usize = 8;

/// O grid do pty-host vive atrás de `Arc<Mutex<Box<dyn VtBackend>>>`; lê sob lock.
impl GridSense for Arc<Mutex<Box<dyn VtBackend>>> {
    fn mode(&self) -> TermMode {
        lock(self).mode()
    }
    fn last_nonempty_line(&self) -> String {
        lock(self).last_nonempty_line()
    }
    /// Região de input REAL: linhas não-vazias do viewport, da ÚLTIMA linha-prompt
    /// (`>`/`❯`, com/sem borda) até o fim — echo de mensagens já submetidas fica ACIMA
    /// do prompt vivo e fora da região. Sem linha-prompt → últimas 8 não-vazias.
    fn input_region_text(&self) -> String {
        let rows: Vec<String> = {
            let g = lock(self);
            let (_, nrows) = g.dims();
            (0..nrows)
                .map(|i| g.row_text(i))
                .filter(|r| !r.trim().is_empty())
                .collect()
        };
        let start = rows
            .iter()
            .rposition(|r| is_prompt_row(r))
            .unwrap_or_else(|| rows.len().saturating_sub(REGION_FALLBACK_ROWS));
        rows[start..].join("\n")
    }
}

// ───────────────────────────── construção do payload ─────────────────────────────

/// Sanitiza o payload de uma colagem A2A para que ele NÃO possa submeter por conta
/// própria — a invariante load-bearing de W0-9 é "o único submit é o
/// [`WriteOp::Submit`] separado". Remove:
/// - os marcadores de bracketed-paste (`ESC[200~`/`ESC[201~`): o `ESC[201~` é o vetor
///   do CVE-2021-31701 (fecharia o bloco e escaparia da colagem);
/// - o **carriage return** (`\r`, `0x0D`): é literalmente a tecla de Enter; um CR
///   injetado no texto submeteria comando(s) antes do `Submit` (terminal CR injection).
///
/// O `\n` (LF) é tratado por [`build_paste`] conforme o modo (mantido no bracketed,
/// onde é multilinha segura; removido no modo-linha, onde também submeteria).
#[must_use]
pub fn sanitize_paste(text: &str) -> String {
    text.replace(PASTE_END_STR, "")
        .replace(PASTE_BEGIN_STR, "")
        .replace('\r', "")
}

/// Bytes do passo "colar texto" da fila serial. Em bracketed-paste mode, embrulha em
/// `ESC[200~ … ESC[201~` (o `\n` do payload é conteúdo de colagem válido — multilinha);
/// senão (modo-linha/cooked), texto puro **sem nenhum terminador de linha**. SEMPRE
/// passa por [`sanitize_paste`] (sem marcadores, sem `\r`) — o Enter é um
/// [`WriteOp::Submit`] separado (a correção load-bearing).
#[must_use]
pub fn build_paste(text: &str, bracketed: bool) -> Vec<u8> {
    let mut clean = sanitize_paste(text); // sem marcadores de paste e sem CR
    if bracketed {
        // Modo bracketed: o TUI trata o bloco como colagem; LF = multilinha (seguro).
        debug_assert!(!clean.contains('\r'), "sanitize_paste deve remover todo CR");
        let mut out = Vec::with_capacity(clean.len() + PASTE_BEGIN.len() + PASTE_END.len());
        out.extend_from_slice(PASTE_BEGIN);
        out.extend_from_slice(clean.as_bytes());
        out.extend_from_slice(PASTE_END);
        out
    } else {
        // Modo-linha (cooked): um LF também submeteria. Remove `\n` para que o ÚNICO
        // submit seja o `Submit` separado.
        clean = clean.replace('\n', "");
        debug_assert!(
            !clean.contains('\r') && !clean.contains('\n'),
            "modo-linha: payload não pode conter terminador de linha"
        );
        clean.into_bytes()
    }
}

/// F1-0-2: veredito de prontidão sobre a REGIÃO de input — pronto ⇔ alguma linha casa o
/// `prompt_ready_regex` **E** nenhum `busy_marker` está presente. O glifo de prompt
/// sozinho NÃO basta: a TUI do Claude Code mostra `❯ ` o tempo todo (inclusive com
/// mensagem enfileirada — "Press up to edit queued messages"); o que distingue ocupado
/// é o marcador no rodapé ("esc to interrupt"). Função PURA — testada contra réplicas
/// byte-fiéis dos grids reais capturados pela baseline F1-0.
fn ready_now(region: &str, prompt: &Regex, busy_markers: &[String]) -> bool {
    prompt.is_match(region) && !busy_markers.iter().any(|m| region.contains(m.as_str()))
}

fn wait_ready(
    grid: &dyn GridSense,
    prompt: &Regex,
    busy_markers: &[String],
    timeout: Duration,
) -> bool {
    let start = Instant::now();
    loop {
        if ready_now(&grid.input_region_text(), prompt, busy_markers) {
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
    /// Resultado da TENTATIVA de injeção faseada. `ready: true` = prompt estava pronto e o texto
    /// **foi injetado**; `ready: false` (ACHADO-2) = a fase de prontidão NÃO passou → **NADA foi
    /// escrito** (alvo OCUPADO) e o chamador deve RETER/re-tentar (o router reclassifica como
    /// retenção via [`DeliveryOutcome::injected`], jamais falha/DLQ). `bracketed` = modo do alvo
    /// (irrelevante quando `ready:false`, pois nada foi colado).
    Injected { ready: bool, bracketed: bool },
    /// `delivery = session_resume`: fallback headless (stub W0-9; em produção seria
    /// `claude --resume <id>` / `codex exec resume`).
    SessionResume,
}

impl DeliveryOutcome {
    /// `true` se a entrega EFETIVAMENTE colocou a mensagem no alvo (texto injetado ou sessão
    /// retomada). `false` SÓ para `Injected{ready:false}` — a fase de prontidão falhou, NADA foi
    /// escrito, o alvo está ocupado. O router usa isto para distinguir entrega-feita de
    /// reter-e-re-tentar (ACHADO-2), sem confundir OCUPADO com FALHA de entrega.
    #[must_use]
    pub fn injected(&self) -> bool {
        match self {
            DeliveryOutcome::Injected { ready, .. } => *ready,
            DeliveryOutcome::SessionResume => true,
        }
    }
}

/// Política de allow-list de injeção (gancho de segurança — A2A é vetor de
/// prompt-injection agente↔agente). Default permissivo no MVP headless, mas presente.
#[derive(Debug, Clone, Copy)]
pub enum InjectPolicy<'a> {
    /// Qualquer nó pode injetar em qualquer nó (testes/headless; produção usa [`WorkspaceTrust`]).
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

/// Matriz de confiança de injeção A2A derivada da **topologia viva do Espaço** (invariante
/// #5 — *pertencimento = conexão*). Segura os pares `(from, target)` confiados e produz uma
/// [`InjectPolicy::AllowOnly`] emprestada. É o caminho de **produção** que LIGA o gancho de
/// segurança: até W5-5 os call-sites do app passavam `InjectPolicy::AllowAll`, deixando a
/// allow-list como dead code (qualquer agente injetava em qualquer agente).
///
/// **Default-deny.** Um par cujo `from` ou `target` não pertença ao Espaço (id
/// desconhecido/forjado, ou de outro Espaço) nunca é gerado → [`InjectPolicy::allows`] o
/// nega. O diálogo legítimo A→B→A não regride: ambos pertencem ao mesmo Espaço, logo `(A,B)`
/// e `(B,A)` estão na lista.
///
/// É um tipo **owned** (segura o `Vec`) porque `InjectPolicy<'a>` *empresta* os pares;
/// mantenha a `WorkspaceTrust` viva enquanto chama [`deliver_a2a`] com [`Self::policy`].
#[derive(Debug, Clone)]
pub struct WorkspaceTrust {
    pairs: Vec<(NodeId, NodeId)>,
}

impl WorkspaceTrust {
    /// Deriva a confiança dos `members` **vivos** do mesmo Espaço: todo par ordenado de nós
    /// **distintos** `(from, target)` é confiado (injeção em si mesmo fica de fora). Um nó
    /// fora de `members` não aparece em par algum → suas injeções são negadas (default-deny).
    #[must_use]
    pub fn from_members(members: &[NodeId]) -> Self {
        let mut pairs = Vec::with_capacity(members.len().saturating_mul(members.len()));
        for &from in members {
            for &target in members {
                if from != target {
                    pairs.push((from, target));
                }
            }
        }
        Self { pairs }
    }

    /// A [`InjectPolicy`] emprestada para passar a [`deliver_a2a`]. `self` deve viver durante
    /// a chamada (a política referencia os pares de dentro).
    #[must_use]
    pub fn policy(&self) -> InjectPolicy<'_> {
        InjectPolicy::AllowOnly(&self.pairs)
    }
}

/// **W0-9.** Entrega `text` de `from` ao terminal vivo `target`, faseado, pela fila
/// serial do supervisor. `grid` é a leitura do grid do alvo (mode + prompt). `policy`
/// é a allow-list; em produção derive-a da topologia viva com [`WorkspaceTrust`]
/// (default-deny par fora do Espaço), reservando `InjectPolicy::AllowAll` a testes/headless.
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

    // 1) wait_ready: espera o prompt REAL do alvo dentro do orçamento do perfil
    //    (`ready_timeout_ms`, default 2s). F1-0-2: timeout → **NÃO injeta** — injetar
    //    "mesmo assim" num alvo ocupado era o mecanismo do texto-colado (18/20 na baseline).
    //    ACHADO-2 (a evolução prometida "para retenção por estado"): prompt-não-pronto NÃO é
    //    FALHA — é sinal de OCUPADO (a TUI está num turno, mesmo que o lifecycle a julgue Idle).
    //    Devolve `Ok(Injected{ready:false})` (= ATTEMPT que NADA escreveu) para o router RETER
    //    (`MessageRetained`/reter→drain), JAMAIS strike→DLQ. A não-injeção (anti-texto-colado)
    //    é PRESERVADA: zero write. `ready` carrega a verdade (era vestigial/sempre-true; ninguém
    //    o lia). A reclassificação retenção-vs-falha mora no router (`DeliveryOutcome::injected`).
    let prompt =
        Regex::new(&profile.prompt_ready_regex).map_err(|e| A2aError::BadRegex(e.to_string()))?;
    let budget = profile.ready_timeout();
    if !wait_ready(grid, &prompt, &profile.busy_markers, budget) {
        return Ok(DeliveryOutcome::Injected {
            ready: false,
            bracketed: false,
        });
    }
    let ready = true; // só se injeta com prontidão verificada (ready:false acima = não injetou)

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

    /// W0-9 segurança (terminal CR injection): um payload com `\r` (e, no modo-linha,
    /// `\n`) NUNCA produz um byte de submit — só o `WriteOp::Submit` separado submete.
    #[test]
    fn cr_lf_injection_cannot_self_submit() {
        // CR é sempre removido (é a tecla de Enter).
        assert_eq!(sanitize_paste("cmd\r"), "cmd");
        assert_eq!(sanitize_paste("a\r\nb"), "a\nb"); // CR fora; LF decidido por build_paste

        // bracketed: CR sumiu, LF preservado (colagem multilinha válida e segura).
        let braq = build_paste("linha1\r\nlinha2", true);
        assert!(!braq.contains(&b'\r'), "sem CR no payload bracketed");
        assert!(
            braq.contains(&b'\n'),
            "LF preservado em bracketed (multilinha)"
        );

        // modo-linha (não bracketed): nem CR nem LF — nada pode pré-submeter.
        let plain = build_paste("benigno\r\nrm -rf /\n", false);
        assert!(
            !plain.contains(&b'\r') && !plain.contains(&b'\n'),
            "modo-linha: nenhum terminador de linha pode pré-submeter"
        );
        assert_eq!(plain, b"benignorm -rf /");
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

    // ---- W5-5: allow-list de produção (default-deny par fora do Espaço) ----

    /// **W5-5 (allow-list real).** A confiança derivada da topologia viva permite o par do
    /// MESMO Espaço (o diálogo A→B legítimo não regride) e NEGA, por construção, um `from`
    /// que não pertence ao Espaço (id desconhecido/forjado) — default-deny.
    #[test]
    #[serial]
    fn inject_policy_workspace_denies_foreign_pair() {
        let sup = Supervisor::new();
        let a = sup.register("@A", None, Box::new(std::io::sink()));
        let b = sup.register("@B", None, Box::new(std::io::sink()));
        // `outsider` existe no roster, mas NÃO pertence ao Espaço passado à confiança.
        let outsider = sup.register("@Outsider", None, Box::new(std::io::sink()));
        let g = grid("$ ", true);

        // Matriz de confiança do Espaço {A, B} (outsider fora) — produz `AllowOnly(pares)`.
        let trust = WorkspaceTrust::from_members(&[a, b]);

        // O diálogo A↔B (mesmo Espaço) é confiado nos DOIS sentidos — o loop A→B→A não
        // regride; o `outsider` é negado em qualquer sentido (default-deny).
        let pol = trust.policy();
        assert!(
            pol.allows(a, b) && pol.allows(b, a),
            "ambas as direções do diálogo A↔B devem ser confiadas (anti-regressão A→B→A)"
        );
        assert!(
            !pol.allows(outsider, a) && !pol.allows(a, outsider),
            "par com nó fora do Espaço é negado em qualquer sentido"
        );

        // (1) par do mesmo Espaço A→B: PASSA (anti-regressão do diálogo).
        let ok = deliver_a2a(&sup, b, a, "oi", &idle_profile(), &g, trust.policy())
            .expect("par do mesmo Espaço deve passar");
        assert!(matches!(ok, DeliveryOutcome::Injected { .. }));

        // (2) `from` fora do Espaço (outsider→A): NEGADO, nada escrito no master de A.
        let denied = deliver_a2a(
            &sup,
            a,
            outsider,
            "payload",
            &idle_profile(),
            &g,
            trust.policy(),
        );
        assert!(
            matches!(denied, Err(A2aError::InjectionDenied { .. })),
            "injeção de nó fora do Espaço deve ser negada (default-deny), obteve {denied:?}"
        );
        assert!(
            sup.applied_ops(a).is_empty(),
            "par negado não pode produzir nenhum write no master do alvo"
        );
    }

    /// **W5-5 (CVE-2021-31701, regressão).** Um payload A2A contendo `ESC[201~` tem o
    /// terminador REMOVIDO antes do write no master — o único `ESC[201~` que sobra é o do
    /// wrapper de bracketed-paste. Trava a sanitização faseada de W0-9 contra regressão.
    #[test]
    #[serial]
    fn a2a_sanitizes_bracket_terminator() {
        let sup = Supervisor::new();
        let a = sup.register("@A", None, Box::new(std::io::sink()));
        let b = sup.register("@B", None, Box::new(std::io::sink()));
        let g = grid("$ ", true); // alvo em BRACKETED_PASTE mode
        let trust = WorkspaceTrust::from_members(&[a, b]);

        // Payload hostil: tenta fechar a colagem e escapar (CVE-2021-31701).
        let evil = "revise o diff\x1b[201~ rm -rf /";
        deliver_a2a(&sup, b, a, evil, &idle_profile(), &g, trust.policy())
            .expect("entrega A→B deve ocorrer");

        // AgentText + Submit chegam ao master de B (a fila serial os aplica).
        assert!(poll_until(Dur::from_secs(5), || sup.applied_ops(b).len() == 2));
        let ops = sup.applied_ops(b);
        let WriteOp::AgentText { bytes, .. } = &ops[0] else {
            panic!("op[0] deveria ser AgentText, obteve {:?}", ops[0]);
        };
        // O `ESC[201~` do payload foi removido; só o terminador do wrapper sobra (exatamente 1).
        let n_end = bytes
            .windows(PASTE_END.len())
            .filter(|w| *w == PASTE_END)
            .count();
        assert_eq!(
            n_end, 1,
            "o ESC[201~ do payload deve ser removido — só o do wrapper sobra"
        );
        assert!(
            bytes.ends_with(PASTE_END),
            "o único ESC[201~ é o terminador do wrapper, no fim"
        );
        // E nunca um CR no texto colado (terminal CR injection — o Enter é Submit separado).
        assert!(!bytes.contains(&b'\r'), "o texto colado nunca carrega CR");
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

    /// `wait_ready` casa o `prompt_ready_regex` na REGIÃO de input do grid (F1-0-2).
    #[test]
    fn wait_ready_matches_prompt() {
        let re = Regex::new(r"\$").expect("regex");
        let ready = grid("$ ", false);
        assert!(wait_ready(&ready, &re, &[], Dur::from_millis(50)));

        let not_ready = grid("trabalhando...", false);
        assert!(!wait_ready(&not_ready, &re, &[], Dur::from_millis(50)));
    }

    // ── F1-0-2: prontidão REAL — região de input + busy markers (grids da baseline) ──

    /// Região de baixo de um claude OCUPADO streamando — réplica byte-fiel do grid REAL
    /// capturado pela baseline F1-0 (`baseline-artifacts/grids/stuck-11.txt`): a caixa de
    /// input mostra `❯ ` MESMO ocupado; o que denuncia o ocupado é o rodapé.
    const REAL_BUSY_REGION: &str =
        "❯ \n────────\n  ⏵⏵ bypass permissions on (shift+tab to cycle) · esc to interrupt";
    /// Grid REAL com mensagem ENFILEIRADA (`stuck-15.txt`) — o prompt-glifo aparece E a
    /// linha do prompt carrega o banner de fila (o caso que enganaria o regex sozinho).
    const REAL_QUEUED_REGION: &str = "❯ Press up to edit queued messages\n────────\n  ⏵⏵ bypass permissions on (shift+tab to cycle) · esc to interrupt";
    /// Claude OCIOSO no prompt: rodapé SEM "esc to interrupt" (validar na tela — F1-0-2
    /// critério 2; a forma exata do rodapé idle é a única parte não coberta pelos grids
    /// da baseline, que só capturou alvos presos).
    const REAL_IDLE_REGION: &str = "❯ \n────────\n  ⏵⏵ bypass permissions on (shift+tab to cycle)";

    /// O regex REAL do `profiles/claude-code.toml` (testado contra ele no
    /// `claude_profile_real_file_is_calibrated`).
    const REAL_PROMPT_REGEX: &str = r"(?m)^\s*[>❯]\s";

    fn claude_busy_markers() -> Vec<String> {
        vec![
            "esc to interrupt".to_string(),
            "Press up to edit queued messages".to_string(),
        ]
    }

    /// Perfil com o regex REAL do claude-code + busy markers REAIS + budget curto (teste).
    fn claude_like_profile(ready_timeout_ms: u64) -> CliProfile {
        let src = format!(
            r#"
            id = "claude-like"
            program = "claude"
            delivery = "pty_inject"
            submit_delay_ms = 5
            prompt_ready_regex = '(?m)^\s*[>❯]\s'
            ready_timeout_ms = {ready_timeout_ms}
            busy_markers = ["esc to interrupt", "Press up to edit queued messages"]
            idle_ms = 100
            ask_timeout_ms = 100000
            [end_signal]
            kind = "stream_json"
            event_type = "result"
        "#
        );
        CliProfile::from_toml_str(&src, "<test>").expect("perfil claude-like parseia")
    }

    /// Mock de grid com região FIXA (para fixtures dos grids reais).
    struct FixedGrid {
        region: String,
        bracketed: bool,
    }
    impl GridSense for FixedGrid {
        fn mode(&self) -> TermMode {
            TermMode {
                bracketed_paste: self.bracketed,
                app_cursor: false,
            }
        }
        fn last_nonempty_line(&self) -> String {
            self.region.lines().last().unwrap_or_default().to_string()
        }
        fn input_region_text(&self) -> String {
            self.region.clone()
        }
    }

    /// Região OCIOSA REAL do Claude Code **v2.1.167** (capturada pela prova com claude
    /// vivo do gate F1-0-2): o prompt vivo VAZIO renderiza `❯` + **NBSP** (U+00A0), e o
    /// rodapé idle não tem "esc to interrupt". A v2.1.166 usava espaço comum — a regra
    /// de linha-prompt precisa aceitar os dois (deriva de TUI prevista na baseline §3).
    const REAL_IDLE_REGION_V2167: &str =
        "❯\u{a0}\n────────\n  ⏵⏵ bypass permissions on (shift+tab to cycle) · ← for agents";

    /// `is_prompt_row` reconhece o prompt vivo nas DUAS gerações da TUI: espaço comum
    /// (v2.1.166, grids da baseline) e NBSP (v2.1.167, prova com claude vivo) — sem o
    /// NBSP a âncora da região cai na linha de ECO do histórico (`❯ PROBE…`), que
    /// produz falso "preso" e região errada.
    #[test]
    fn is_prompt_row_accepts_nbsp_live_prompt() {
        assert!(is_prompt_row("❯ "), "espaço comum (v2.1.166)");
        assert!(is_prompt_row("❯\u{a0}"), "NBSP (v2.1.167, claude real)");
        assert!(is_prompt_row("❯\u{a0}texto digitado"), "NBSP com texto");
        assert!(is_prompt_row("│ ❯ x"), "com borda de caixa");
        assert!(
            !is_prompt_row("⏺ Ok, mensagem ignorada."),
            "linha de resposta não"
        );
        assert!(!is_prompt_row("✻ Churned for 5s"), "spinner não");
    }

    /// A região idle REAL da v2.1.167 é julgada PRONTA (regex `\s` casa NBSP; rodapé
    /// idle não carrega busy marker).
    #[test]
    fn ready_now_accepts_real_v2167_idle_region() {
        let re = Regex::new(REAL_PROMPT_REGEX).expect("regex");
        assert!(ready_now(
            REAL_IDLE_REGION_V2167,
            &re,
            &claude_busy_markers()
        ));
    }

    /// `ready_now` julga as regiões REAIS corretamente: ocupado/enfileirado → NÃO pronto
    /// (mesmo com o glifo `❯` visível); ocioso → pronto; sem prompt → não pronto.
    #[test]
    fn ready_now_judges_real_grid_regions() {
        let re = Regex::new(REAL_PROMPT_REGEX).expect("regex real compila");
        let markers = claude_busy_markers();
        assert!(
            !ready_now(REAL_BUSY_REGION, &re, &markers),
            "claude streamando (rodapé 'esc to interrupt') NÃO está pronto"
        );
        assert!(
            !ready_now(REAL_QUEUED_REGION, &re, &markers),
            "mensagem enfileirada NÃO é prontidão — o caso que o regex sozinho erraria"
        );
        assert!(
            ready_now(REAL_IDLE_REGION, &re, &markers),
            "ocioso no prompt → pronto"
        );
        assert!(
            !ready_now("✻ Pondering…\n  trabalhando", &re, &markers),
            "região sem linha-prompt não é prontidão"
        );
    }

    /// **Critério 1 da F1-0-2 (o fix do P0):** alvo fora do prompt → `deliver_a2a` espera
    /// até o budget e, no timeout, **NÃO injeta** (erro explícito; zero `WriteOp` na fila).
    /// Antes do fix: injetava "mesmo assim" (`ready:false`) — exatamente o texto-colado
    /// 18/20 da baseline.
    #[test]
    #[serial]
    fn deliver_not_ready_never_injects() {
        let sup = Supervisor::new();
        let from = sup.register("@A", None, Box::new(std::io::sink()));
        let target = sup.register("@B", None, Box::new(std::io::sink()));
        let busy = FixedGrid {
            region: REAL_BUSY_REGION.to_string(),
            bracketed: true,
        };
        let prof = claude_like_profile(80); // budget curto: o teste não espera 30s

        let res = deliver_a2a(
            &sup,
            target,
            from,
            "PROBE — não deveria entrar",
            &prof,
            &busy,
            InjectPolicy::AllowAll,
        );
        // ACHADO-2: prompt-não-pronto NÃO é falha — é sinal de OCUPADO. Devolve
        // `Ok(Injected{ready:false})` (NADA injetado) para o router RETER (não strike/DLQ).
        assert!(
            matches!(res, Ok(DeliveryOutcome::Injected { ready: false, .. })),
            "prompt não-pronto = OCUPADO (Ok ready:false, retenção), obteve {res:?}"
        );
        std::thread::sleep(Dur::from_millis(50));
        assert!(
            sup.applied_ops(target).is_empty(),
            "alvo não-pronto NUNCA recebe write (era o mecanismo do texto-colado — preservado)"
        );
    }

    /// O caminho feliz do fix: alvo ocupado fica pronto DENTRO do budget → a entrega
    /// espera e injeta faseado (AgentText + Submit) com `ready: true`.
    #[test]
    #[serial]
    fn deliver_waits_until_ready_then_injects() {
        let sup = Supervisor::new();
        let from = sup.register("@A", None, Box::new(std::io::sink()));
        let target = sup.register("@B", None, Box::new(std::io::sink()));

        // Grid compartilhado que COMEÇA ocupado e fica ocioso depois de ~60ms.
        struct SwitchingGrid {
            region: Arc<Mutex<String>>,
        }
        impl GridSense for SwitchingGrid {
            fn mode(&self) -> TermMode {
                TermMode {
                    bracketed_paste: true,
                    app_cursor: false,
                }
            }
            fn last_nonempty_line(&self) -> String {
                lock(&self.region)
                    .lines()
                    .last()
                    .unwrap_or_default()
                    .to_string()
            }
            fn input_region_text(&self) -> String {
                lock(&self.region).clone()
            }
        }
        let region = Arc::new(Mutex::new(REAL_BUSY_REGION.to_string()));
        let g = SwitchingGrid {
            region: Arc::clone(&region),
        };
        let flipper = {
            let region = Arc::clone(&region);
            std::thread::spawn(move || {
                std::thread::sleep(Dur::from_millis(60));
                *lock(&region) = REAL_IDLE_REGION.to_string();
            })
        };

        let prof = claude_like_profile(2_000);
        let res = deliver_a2a(
            &sup,
            target,
            from,
            "agora sim",
            &prof,
            &g,
            InjectPolicy::AllowAll,
        )
        .expect("alvo ficou pronto dentro do budget → entrega");
        assert_eq!(
            res,
            DeliveryOutcome::Injected {
                ready: true,
                bracketed: true
            }
        );
        assert!(poll_until(Dur::from_secs(5), || sup
            .applied_ops(target)
            .len()
            == 2));
        flipper.join().expect("join flipper");
    }

    /// A região de input REAL extraída de um `VtBackend` de verdade: alimenta o backend
    /// com o conteúdo do grid preso da baseline e confere que a região vai da última
    /// linha-prompt ao fim (inclui o rodapé — onde mora o busy marker).
    #[test]
    fn input_region_from_real_vt_backend() {
        let mut b = AlacrittyBackend::new(120, 24);
        b.advance(b"\x1b[2J\x1b[H");
        b.advance("⏺ Recebidas as mensagens de probe — ignoradas.\r\n".as_bytes());
        b.advance("✻ Cogitated for 9s\r\n".as_bytes());
        b.advance("❯ \r\n".as_bytes());
        b.advance("────────\r\n".as_bytes());
        b.advance("  ⏵⏵ bypass permissions on (shift+tab to cycle) · esc to interrupt".as_bytes());
        let g: Arc<Mutex<Box<dyn VtBackend>>> = Arc::new(Mutex::new(Box::new(b)));

        let region = g.input_region_text();
        assert!(
            region.contains("esc to interrupt"),
            "a região deve incluir o rodapé (busy marker): {region:?}"
        );
        let first = region.lines().next().unwrap_or_default();
        assert!(
            first.trim_start().starts_with('❯'),
            "a região começa na última linha-prompt, não no histórico: {region:?}"
        );
        assert!(
            !region.contains("Cogitated"),
            "histórico acima do prompt fica FORA da região"
        );
    }
}
