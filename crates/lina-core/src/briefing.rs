//! F3-2-6 — **Briefing em CAMADAS** do worker (módulo PURO; Hermes §6.1/§6.2).
//!
//! O bootstrap de turno-0 (`lina whoami`, `lina-bootstrap`) entrega a identidade mínima; este
//! módulo monta o briefing **estruturado por volatilidade** que um terminal lê no boot de um turno
//! de trabalho. A ordem é deliberada (Hermes §6.1): o que muda RARAMENTE no topo, o que muda SEMPRE
//! no fim — o oposto do "muro de contexto genérico" (objetivo a do despacho).
//!
//! - **stable** — papel + invariantes do produto + verbos/skills. Muda só com o produto.
//! - **context** — plano compartilhado + Decisões vivas + âncoras do projeto. Muda por sessão.
//! - **volatile** — estado vivo (colegas/claims) + a DATA (date-only). Muda a cada boot.
//!
//! ## Invariantes que este módulo honra
//! - **Texto acima do CLI (inv #1):** ZERO LLM — é montagem determinística de strings, não geração.
//! - **Projeção do log (inv #4):** o chamador alimenta as camadas a partir do event log / plano /
//!   roster (o módulo não faz I/O — recebe os dados já projetados, então é PURO e testável).
//! - **Gating por papel (Hermes §6.1: worker vs orquestrador):** o **protocolo de orquestração**
//!   (decompor/despachar/monitorar) só entra para quem ORQUESTRA. Um FRONTEND não recebe o protocolo
//!   de orquestração — receberia ruído que não usa, e o despacho crava esse gate como critério.
//! - **K-cap (anti-muro):** as listas de contexto são limitadas a [`CONTEXT_ITEM_CAP`]; a truncação é
//!   SEMPRE sinalizada (nunca corte silencioso — o terminal sabe que há mais e por onde puxar).

/// Teto de itens por lista de CONTEXTO (K-cap). O briefing **informa**, não despeja: além disto, o
/// terminal puxa sob demanda (`lina plan read`). Bounded ⇒ o briefing não cresce sem limite com o log.
pub const CONTEXT_ITEM_CAP: usize = 12;

/// As entradas VARIÁVEIS das três camadas — projetadas pelo chamador (log/plano/roster) e passadas
/// já prontas, para o módulo permanecer PURO (sem I/O) e testável. As partes CONSTANTES do produto
/// (invariantes, verify-loop, protocolo) são texto fixo do próprio módulo.
#[derive(Debug, Clone)]
pub struct BriefingInput<'a> {
    /// Papel CRU do terminal (do roster). Decide o gating: [`role_orchestrates`] → recebe o protocolo.
    pub role: &'a str,
    /// stable — skills/verbos do papel (ex.: `senior-frontend`, `lina-agent-bus`).
    pub skills: &'a [String],
    /// context — itens do plano compartilhado atribuíveis/relevantes ao terminal.
    pub plan_items: &'a [String],
    /// context — Decisões vivas do plano (`plan.md → Decisões`) que regem o trabalho.
    pub decisions: &'a [String],
    /// volatile — estado vivo (colega → status; claims abertos). Re-lido a cada boot.
    pub live_state: &'a [String],
    /// volatile — a DATA, **date-only** (`YYYY-MM-DD`). Date-only é deliberado: a hora muda a cada
    /// segundo (instabilidade/ruído sem sinal); a data dá frescor sem poluir nem desestabilizar o
    /// briefing entre dois boots do mesmo dia (replay determinístico).
    pub date: &'a str,
}

/// Os sete invariantes inegociáveis do produto (CLAUDE.md) — a espinha ESTÁVEL do briefing. Forma
/// curta: o terminal já tem a doutrina completa; aqui é o lembrete que não deixa quebrar uma porta.
const INVARIANTES: &[&str] = &[
    "1. Sem LLM/harness/chat próprios — orquestramos CLIs de terceiros; o chat é o terminal.",
    "2. Local-first — nada sai da máquina por padrão; exposição é opt-in sinalizado.",
    "3. Neutralidade multi-CLI — trocar de CLI é trocar um CLI Profile (TOML), nunca refém.",
    "4. O event log é a fonte da verdade — layout/buffers/tabelas são projeções reconstruíveis.",
    "5. Pertencimento = conexão — a topologia deriva de quem está no Espaço, não de cabos.",
    "6. Não-técnico-first — zero jargão na superfície; estado sempre salvo e visível.",
    "7. Core/shell split — o core Rust é o ativo durável; a UI é substituível (trait UiHost).",
];

/// `true` se o papel **orquestra** (roda o loop: decompõe → despacha → monitora → corrige). Só o
/// orquestrador recebe o protocolo de orquestração na camada estável; todo papel de WORKER (FRONTEND,
/// BACKEND, QA, DEVELOPER, …) executa e NÃO o recebe. Allowlist explícita (default-deny — um papel
/// novo é worker até prova em contrário, mais seguro que vazar o protocolo por engano). Tolerante a
/// `@`/caixa/espaços, espelhando o `normalize_name` do roster vivo.
#[must_use]
pub fn role_orchestrates(role: &str) -> bool {
    let norm = role
        .trim()
        .trim_start_matches('@')
        .trim()
        .to_ascii_uppercase();
    matches!(norm.as_str(), "MAESTRO" | "ORQUESTRADOR")
}

/// Monta o briefing em camadas para `input`, **gated por papel**. Função PURA: mesma entrada → mesma
/// saída (sem relógio, sem I/O). A ordem das camadas (estável → contexto → volátil) e o gating do
/// protocolo de orquestração são o contrato testado (gate (e) da onda F3-2).
#[must_use]
pub fn build_briefing(input: &BriefingInput) -> String {
    let mut out = String::new();
    out.push_str("=== BRIEFING (camadas) ===\n");

    // ───────── ESTÁVEL — quem você é e as regras que não mudam ─────────
    out.push_str("\n[ESTAVEL] papel e regras que nao mudam\n");
    out.push_str(&format!("papel: {}\n", display_role(input.role)));
    out.push_str(
        "invariantes (nao quebre — antes de decidir, pergunte: isto fecha uma porta acima?):\n",
    );
    for inv in INVARIANTES {
        out.push_str(&format!("  {inv}\n"));
    }
    out.push_str(&format!("skills/verbos: {}\n", join_or_dash(input.skills)));
    if role_orchestrates(input.role) {
        // GATING (Hermes §6.1): só o orquestrador recebe o protocolo — o worker executa, não coordena.
        out.push_str(
            "protocolo de orquestracao: decomponha no plano (lina plan), despache com contexto\n  \
             (lina handoff/broadcast), monitore o trajeto (lina check / lina history) e corrija\n  \
             informado no FAIL. Voce e DONO do resultado, nao executor de pedido.\n",
        );
    }
    // Verify-loop no boot (Hermes §6.2): re-check before acting + evidência antes de "pronto".
    out.push_str(
        "verify-loop: ANTES de agir, re-cheque o estado (re-leia o plano/log; rode `cargo test -p`/\n  \
         clippy e `git status` quando tocar codigo). DEPOIS, so diga \"pronto\" com EVIDENCIA\n  \
         observada (rodou, leu o output, viu o comportamento) — nunca por suposicao.\n",
    );

    // ───────── CONTEXTO — o projeto e as decisões vivas (K-capped) ─────────
    out.push_str("\n[CONTEXTO] o projeto e as decisoes vivas\n");
    out.push_str("plano (itens):\n");
    push_capped(&mut out, input.plan_items, "lina plan read");
    out.push_str("decisoes vivas:\n");
    push_capped(&mut out, input.decisions, "lina plan read");

    // ───────── VOLÁTIL — o estado de agora (re-leia a cada boot) ─────────
    out.push_str(&format!("\n[VOLATIL] estado de agora — {}\n", input.date));
    out.push_str("time / claims:\n");
    push_capped(&mut out, input.live_state, "lina list");

    out.push_str("=== FIM BRIEFING ===");
    out
}

/// Exibe o papel de forma legível (CRU se vazio → marcador honesto, nunca um papel inventado).
fn display_role(role: &str) -> &str {
    let r = role.trim();
    if r.is_empty() {
        "(sem papel definido)"
    } else {
        r
    }
}

/// Lista separada por `, ` ou `—` quando vazia (honesto: vazio não vira lista falsa).
fn join_or_dash(items: &[String]) -> String {
    if items.is_empty() {
        "—".to_string()
    } else {
        items.join(", ")
    }
}

/// Empilha até [`CONTEXT_ITEM_CAP`] itens; trunca SINALIZANDO o resto (anti corte silencioso — o
/// terminal sabe que há mais e por onde puxar). Lista vazia → marcador honesto, nunca uma linha falsa.
fn push_capped(out: &mut String, items: &[String], pull_hint: &str) {
    if items.is_empty() {
        out.push_str("  — (nada aqui agora)\n");
        return;
    }
    for item in items.iter().take(CONTEXT_ITEM_CAP) {
        out.push_str(&format!("  - {item}\n"));
    }
    if items.len() > CONTEXT_ITEM_CAP {
        let rest = items.len() - CONTEXT_ITEM_CAP;
        out.push_str(&format!("  … (+{rest} — puxe o resto com `{pull_hint}`)\n"));
    }
}

// ───────────────────────────────────── testes ─────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    fn sample(role: &str) -> String {
        let skills = lines(&["senior-frontend", "lina-agent-bus"]);
        let plan = lines(&["T1 :: montar a tela :: @owner:voce"]);
        let dec = lines(&["UI usa tokens semanticos, nunca cor hard-coded"]);
        let live = lines(&["@Terminal A: Busy", "@Terminal B: Idle"]);
        build_briefing(&BriefingInput {
            role,
            skills: &skills,
            plan_items: &plan,
            decisions: &dec,
            live_state: &live,
            date: "2026-06-18",
        })
    }

    /// Critério (gate e): as TRÊS camadas são montadas, nesta ordem (estável → contexto → volátil).
    #[test]
    fn assembles_three_layers_in_order() {
        let b = sample("FRONTEND");
        let s = b.find("[ESTAVEL]").expect("camada estável");
        let c = b.find("[CONTEXTO]").expect("camada contexto");
        let v = b.find("[VOLATIL]").expect("camada volátil");
        assert!(
            s < c && c < v,
            "ordem por volatilidade: estável → contexto → volátil"
        );
        // O conteúdo de cada camada está presente.
        assert!(b.contains("papel: FRONTEND"));
        assert!(b.contains("T1 :: montar a tela"), "contexto traz o plano");
        assert!(
            b.contains("@Terminal A: Busy"),
            "volátil traz o estado vivo"
        );
        assert!(
            b.contains("2026-06-18"),
            "volátil carimba a data (date-only)"
        );
    }

    /// Critério (gate e) DURO: FRONTEND **não** recebe o protocolo de orquestração; o orquestrador
    /// recebe. Mesma entrada, só o papel muda — isola o gating.
    #[test]
    fn frontend_does_not_get_orchestration_protocol_but_orchestrator_does() {
        let worker = sample("FRONTEND");
        assert!(
            !worker.contains("protocolo de orquestracao"),
            "worker NAO recebe o protocolo de orquestracao"
        );
        // mas o resto da camada estável (papel, invariantes, verify-loop) ele recebe.
        assert!(worker.contains("invariantes"));
        assert!(worker.contains("verify-loop"));

        let maestro = sample("MAESTRO");
        assert!(
            maestro.contains("protocolo de orquestracao"),
            "o orquestrador recebe o protocolo"
        );
    }

    /// `role_orchestrates`: só o orquestrador (tolerante a @/caixa/espaços); todo worker é default-deny.
    #[test]
    fn role_orchestrates_is_allowlist_default_deny() {
        for yes in ["MAESTRO", "maestro", " @Maestro ", "Orquestrador"] {
            assert!(role_orchestrates(yes), "{yes:?} orquestra");
        }
        for no in [
            "FRONTEND",
            "BACKEND",
            "QA",
            "DEVELOPER",
            "ARQUITETO",
            "",
            "tradutor",
        ] {
            assert!(!role_orchestrates(no), "{no:?} é worker (default-deny)");
        }
    }

    /// K-cap: contexto além de [`CONTEXT_ITEM_CAP`] é truncado COM sinalização (nunca corte silencioso).
    #[test]
    fn context_is_k_capped_and_truncation_is_signalled() {
        let many: Vec<String> = (0..CONTEXT_ITEM_CAP + 5)
            .map(|i| format!("T{i} :: item :: @owner:?"))
            .collect();
        let b = build_briefing(&BriefingInput {
            role: "BACKEND",
            skills: &[],
            plan_items: &many,
            decisions: &[],
            live_state: &[],
            date: "2026-06-18",
        });
        // Exatamente CONTEXT_ITEM_CAP itens de plano renderizados + a linha de "+N".
        let rendered = b.matches("  - T").count();
        assert_eq!(rendered, CONTEXT_ITEM_CAP, "nunca mais que o teto");
        assert!(
            b.contains("(+5 — puxe o resto com `lina plan read`)"),
            "truncação sinalizada"
        );
        // Listas vazias degradam honestamente (sem linha falsa).
        assert!(
            b.contains("— (nada aqui agora)"),
            "vazio é honesto, não inventa item"
        );
    }

    /// Pureza: mesma entrada → byte-idêntico (sem relógio/IO escondido) — base do replay determinístico.
    #[test]
    fn build_is_pure() {
        assert_eq!(sample("FRONTEND"), sample("FRONTEND"));
    }

    /// Papel vazio degrada honestamente (marcador, nunca um papel inventado).
    #[test]
    fn empty_role_degrades_honestly() {
        let b = sample("");
        assert!(b.contains("papel: (sem papel definido)"));
        assert!(
            !b.contains("protocolo de orquestracao"),
            "sem papel = worker (default-deny)"
        );
    }
}
