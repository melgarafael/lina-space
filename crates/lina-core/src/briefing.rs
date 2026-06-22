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
//!
//! ## Camadas de CONTEXTO da onda F3-5 (todas gated, todas DADO-nunca-autoridade)
//! - **Pistas (F3-5-6, doc-fonte 65):** os `paths` que a IA enxerga no projeto deste terminal,
//!   lidos da projeção [`crate::clue::ClueSet`] e **gated por escopo** (`paths_for(scope)`).
//! - **Doutrinas-como-hooks (F3-5-9, doc-fonte 61):** no foco "app/sistema" (`dev_app`), injeta as
//!   doutrinas de arquitetura/segurança/código (anti-slop) — **gated por foco**, texto acima do CLI.
//! - **Skills (camada de skills):** as skills escolhidas para o nó, lidas da projeção
//!   [`SelectedSkills`] de `SkillSelected` (emitido por J) — **desacoplado**: lemos a projeção, não
//!   o código de J.

use std::collections::BTreeMap;

use crate::clue::ClueSet;
use crate::events::{DomainEvent, EventRecord, EventStore, StoreError};

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

    /// F3-5-6 — escopo/projeto deste terminal; casa com as pistas (`clues.paths_for(scope)`). O
    /// gating por escopo garante que o terminal só enxergue as pistas do SEU projeto.
    pub scope: &'a str,
    /// F3-5-6 — projeção das pistas (`ClueSet`), LIDA aqui. Remover a pista (redefinir vazia) → a
    /// projeção retrai o escopo → o `paths` some do briefing no próximo turno.
    pub clues: &'a ClueSet,
    /// F3-5-9 — foco do Espaço/terminal. "app/sistema" (`dev_app`) ATIVA as doutrinas-como-hooks;
    /// qualquer outro foco → não entram (gating por foco, default-deny via [`focus_builds_software`]).
    pub focus: &'a str,
    /// id do nó (casa com `SkillSelected.node`) — para a camada de skills resolver as do terminal.
    pub node: &'a str,
    /// camada de skills — projeção de `SkillSelected` (emitido por J), LIDA aqui (gated por nó). O
    /// briefing lê a projeção, nunca o código de J (doutrina "fronteira em camadas").
    pub selected_skills: &'a SelectedSkills,
}

/// Uma skill escolhida para um nó, reconstruída de `SkillSelected` (F3-5-4). Os campos são DADO de
/// adoção/contexto — `source`/`trigger` informam de onde/por que a skill entrou, nunca autoridade.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedSkill {
    /// Nome da skill (ex.: `senior-backend`).
    pub skill: String,
    /// Gatilho que casou (palavra-chave/contexto), se houver.
    pub trigger: Option<String>,
    /// De onde a skill veio (catálogo/hub/fábrica).
    pub source: String,
}

/// Projeção das skills selecionadas por NÓ (replay de `SkillSelected`). Acumula por nó; o último
/// `SkillSelected` da MESMA skill no mesmo nó vence (atualiza gatilho/origem). `BTreeMap` aninhado
/// dá ordem estável → briefing determinístico. PURA sobre o log (inv #4), como `ClueSet`/`Mentality`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SelectedSkills {
    by_node: BTreeMap<String, BTreeMap<String, SelectedSkill>>,
}

impl SelectedSkills {
    /// Reconstrói a projeção do `EventStore` por replay.
    ///
    /// # Errors
    /// Falha ao ler o event log.
    pub fn replay(store: &EventStore) -> Result<Self, StoreError> {
        Ok(Self::from_records(&store.events()?))
    }

    /// **Coração determinístico (PURO).** Varre os registros e agrupa as skills escolhidas por nó.
    #[must_use]
    pub fn from_records(records: &[EventRecord]) -> Self {
        let mut by_node: BTreeMap<String, BTreeMap<String, SelectedSkill>> = BTreeMap::new();
        for record in records {
            // Indecodificável → pula (projeção DERIVADA, não validadora — padrão `ClueSet`/`Mentality`).
            let Ok(event) =
                DomainEvent::from_record(&record.kind, record.version, record.payload.clone())
            else {
                continue;
            };
            if let DomainEvent::SkillSelected {
                node,
                skill,
                trigger,
                source,
            } = event
            {
                // Último-vence por (nó, skill): re-selecionar a mesma skill atualiza gatilho/origem,
                // sem duplicar.
                by_node.entry(node).or_default().insert(
                    skill.clone(),
                    SelectedSkill {
                        skill,
                        trigger,
                        source,
                    },
                );
            }
        }
        Self { by_node }
    }

    /// As skills escolhidas para `node`, em ordem estável (vazio se nenhuma). Gating por nó.
    #[must_use]
    pub fn skills_for(&self, node: &str) -> Vec<&SelectedSkill> {
        self.by_node
            .get(node)
            .map(|m| m.values().collect())
            .unwrap_or_default()
    }
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

/// `true` se o FOCO do Espaço é "desenvolver app/sistema" — o gatilho das doutrinas-como-hooks
/// (doc-fonte 61). Casa com o id de contrato `dev_app` (`FocusPreset::DevApp`, gravado em
/// `WorkspaceCreated.focus_preset`) e com as formas humanas equivalentes. Allowlist explícita
/// (default-deny: `research_content`/`blank`/`geral`/"" → sem doutrina), espelhando
/// [`role_orchestrates`]. Tolerante a `@`/caixa/espaços.
#[must_use]
pub fn focus_builds_software(focus: &str) -> bool {
    let norm = focus
        .trim()
        .trim_start_matches('@')
        .trim()
        .to_ascii_lowercase();
    matches!(norm.as_str(), "dev_app" | "app/sistema" | "app" | "sistema")
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

    // ── DOUTRINAS-COMO-HOOKS (F3-5-9, doc-fonte 61) — gated por FOCO, texto acima do CLI (inv #1) ──
    // No foco "app/sistema" (`dev_app`), as doutrinas seniores de arquitetura/segurança/código entram
    // automaticamente (anti-slop onde mais importa: a base de longo prazo do projeto). Fora do foco,
    // não entram — seriam ruído. ZERO LLM: é texto fixo do produto, montado por gating determinístico.
    if focus_builds_software(input.focus) {
        out.push_str(&format!(
            "\n[DOUTRINA] doutrina app/sistema — aplique como {} senior (anti-slop em arquitetura e seguranca):\n",
            display_role(input.role)
        ));
        out.push_str(
            "  arquitetura: antes de qualquer decisao, pergunte \"isto fecha uma porta acima?\" — se\n  \
             sim, PARE e registre um ADR curto; nada de abstracao especulativa nem refactor de brinde.\n",
        );
        out.push_str(
            "  seguranca: nenhum campo escrito por agente (from/payload/contrato/filename) decide\n  \
             identidade, ordem ou autorizacao — contrato e DADO, jamais autoridade; acao irreversivel\n  \
             exige gate humano.\n",
        );
        out.push_str(
            "  codigo: causa raiz acima de fix temporario; zero erro engolido (catch vazio / unwrap em\n  \
             producao); nomes dizem o QUE; o obvio/morno (slop) nao passa.\n",
        );
    }

    // ───────── CONTEXTO — o projeto e as decisões vivas (K-capped) ─────────
    out.push_str("\n[CONTEXTO] o projeto e as decisoes vivas\n");
    out.push_str("plano (itens):\n");
    push_capped(&mut out, input.plan_items, "lina plan read");
    out.push_str("decisoes vivas:\n");
    push_capped(&mut out, input.decisions, "lina plan read");

    // Pistas (F3-5-6, doc-fonte 65) — gated por ESCOPO: só entra a pista ATIVA do projeto deste
    // terminal. Removida (redefinida vazia) → a projeção retrai o escopo → a seção some no próximo
    // turno. `paths` é DADO de contexto (o que a IA olha), nunca autoridade de acesso.
    let clue_paths = input.clues.paths_for(input.scope);
    if !clue_paths.is_empty() {
        out.push_str("pistas (arquivos/pastas que a IA enxerga neste projeto):\n");
        for path in clue_paths {
            out.push_str(&format!("  - {path}\n"));
        }
    }

    // Skills (camada de skills) — gated por NÓ: as skills escolhidas para este terminal, lidas da
    // projeção `SelectedSkills` de `SkillSelected` (emitido por J). Desacoplado: lemos a projeção.
    let node_skills = input.selected_skills.skills_for(input.node);
    if !node_skills.is_empty() {
        out.push_str("skills escolhidas para este terminal:\n");
        for s in node_skills {
            match &s.trigger {
                Some(t) => out.push_str(&format!("  - {} (gatilho: {t})\n", s.skill)),
                None => out.push_str(&format!("  - {}\n", s.skill)),
            }
        }
    }

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
        let clues = ClueSet::default();
        let selected = SelectedSkills::default();
        build_briefing(&BriefingInput {
            role,
            skills: &skills,
            plan_items: &plan,
            decisions: &dec,
            live_state: &live,
            date: "2026-06-18",
            scope: "",
            clues: &clues,
            focus: "", // sem foco app/sistema → sem doutrinas (isola os testes legados)
            node: "",
            selected_skills: &selected,
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
        let clues = ClueSet::default();
        let selected = SelectedSkills::default();
        let b = build_briefing(&BriefingInput {
            role: "BACKEND",
            skills: &[],
            plan_items: &many,
            decisions: &[],
            live_state: &[],
            date: "2026-06-18",
            scope: "",
            clues: &clues,
            focus: "",
            node: "",
            selected_skills: &selected,
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

    // ───────── camadas de contexto da F3-5 (pistas · doutrinas-como-hooks · skills) ─────────

    /// Monta um briefing variando só os parâmetros contextuais (escopo/pistas/foco/nó/skills).
    fn briefing_with(
        scope: &str,
        clues: &ClueSet,
        focus: &str,
        node: &str,
        selected: &SelectedSkills,
    ) -> String {
        build_briefing(&BriefingInput {
            role: "BACKEND",
            skills: &[],
            plan_items: &[],
            decisions: &[],
            live_state: &[],
            date: "2026-06-18",
            scope,
            clues,
            focus,
            node,
            selected_skills: selected,
        })
    }

    fn clue_record(seq: u64, scope: &str, paths: &[&str]) -> EventRecord {
        let event = DomainEvent::ClueSetDefined {
            scope: scope.to_string(),
            paths: paths.iter().map(|p| (*p).to_string()).collect(),
            label: None,
        };
        EventRecord {
            seq,
            ts: seq,
            kind: event.kind().to_string(),
            version: event.current_version(),
            payload: serde_json::to_value(&event).expect("serializa"),
        }
    }

    fn skill_record(
        seq: u64,
        node: &str,
        skill: &str,
        trigger: Option<&str>,
        source: &str,
    ) -> EventRecord {
        let event = DomainEvent::SkillSelected {
            node: node.to_string(),
            skill: skill.to_string(),
            trigger: trigger.map(str::to_string),
            source: source.to_string(),
        };
        EventRecord {
            seq,
            ts: seq,
            kind: event.kind().to_string(),
            version: event.current_version(),
            payload: serde_json::to_value(&event).expect("serializa"),
        }
    }

    /// Critério (c): pista definida → o terminal daquele projeto enxerga os arquivos no briefing.
    #[test]
    fn clue_paths_appear_in_briefing_for_scope() {
        let clues =
            ClueSet::from_records(&[clue_record(1, "checkout", &["src/checkout/", "docs/x"])]);
        let b = briefing_with("checkout", &clues, "", "", &SelectedSkills::default());
        assert!(b.contains("src/checkout/"), "a pista entra no contexto");
        assert!(b.contains("docs/x"));
        assert!(b.contains("pistas"), "a camada de pistas é rotulada");
    }

    /// Critério (c): remover a pista (redefinir vazia) → some do briefing no próximo turno.
    #[test]
    fn clue_disappears_from_briefing_when_removed() {
        let clues = ClueSet::from_records(&[
            clue_record(1, "checkout", &["src/checkout/"]),
            clue_record(2, "checkout", &[]), // remoção
        ]);
        let b = briefing_with("checkout", &clues, "", "", &SelectedSkills::default());
        assert!(
            !b.contains("src/checkout/"),
            "pista removida não aparece mais no briefing"
        );
        assert!(
            !b.contains("pistas (arquivos"),
            "sem pista ativa → a camada de pistas nem é rotulada (some)"
        );
    }

    /// Gating por escopo: o terminal de OUTRO escopo não enxerga a pista (zero vazamento).
    #[test]
    fn clue_is_scoped_in_briefing() {
        let clues = ClueSet::from_records(&[clue_record(1, "checkout", &["src/checkout/"])]);
        let b = briefing_with("landing", &clues, "", "", &SelectedSkills::default());
        assert!(
            !b.contains("src/checkout/"),
            "o escopo 'landing' não vê a pista de 'checkout'"
        );
    }

    /// Critério (c): doutrinas-como-hooks entram no foco "app/sistema" e NÃO entram fora dele.
    #[test]
    fn doctrines_gated_by_app_focus() {
        let with_focus = briefing_with(
            "",
            &ClueSet::default(),
            "dev_app",
            "",
            &SelectedSkills::default(),
        );
        assert!(
            with_focus.contains("doutrina app/sistema"),
            "no foco app/sistema as doutrinas entram"
        );
        assert!(
            with_focus.contains("fecha uma porta"),
            "doutrina de arquitetura presente"
        );
        assert!(
            with_focus.contains("contrato e DADO, jamais autoridade"),
            "doutrina de segurança presente (fiel ao invariante)"
        );

        let no_focus = briefing_with(
            "",
            &ClueSet::default(),
            "research_content",
            "",
            &SelectedSkills::default(),
        );
        assert!(
            !no_focus.contains("doutrina app/sistema"),
            "fora do foco app/sistema as doutrinas NÃO entram"
        );
    }

    /// `focus_builds_software`: allowlist (dev_app + formas humanas); default-deny no resto.
    #[test]
    fn focus_builds_software_is_allowlist_default_deny() {
        for yes in ["dev_app", "app/sistema", "App", " @sistema "] {
            assert!(focus_builds_software(yes), "{yes:?} é foco de app/sistema");
        }
        for no in ["research_content", "blank", "geral", "", "marketing"] {
            assert!(!focus_builds_software(no), "{no:?} não dispara doutrinas");
        }
    }

    /// Camada de skills: o briefing LÊ a projeção `SelectedSkills` e injeta as skills do nó.
    #[test]
    fn selected_skills_layer_reads_projection_gated_by_node() {
        let selected = SelectedSkills::from_records(&[skill_record(
            1,
            "n1",
            "senior-backend",
            Some("api"),
            "catalogo",
        )]);
        let b1 = briefing_with("", &ClueSet::default(), "", "n1", &selected);
        assert!(
            b1.contains("senior-backend"),
            "o nó n1 vê a skill escolhida"
        );
        assert!(b1.contains("skills escolhidas"), "a camada é rotulada");

        let b2 = briefing_with("", &ClueSet::default(), "", "n2", &selected);
        assert!(
            !b2.contains("senior-backend"),
            "outro nó não vê a skill de n1 (gating por nó)"
        );
    }

    /// Projeção `SelectedSkills`: agrupa por nó; a MESMA skill re-selecionada no nó atualiza (último
    /// vence), não duplica.
    #[test]
    fn selected_skills_projection_groups_by_node_last_wins() {
        let selected = SelectedSkills::from_records(&[
            skill_record(1, "n1", "senior-backend", Some("api"), "catalogo"),
            skill_record(2, "n1", "lina-agent-bus", None, "catalogo"),
            skill_record(3, "n1", "senior-backend", Some("query"), "hub"), // mesma skill, atualiza
            skill_record(4, "n2", "senior-frontend", None, "catalogo"),
        ]);
        let n1 = selected.skills_for("n1");
        assert_eq!(n1.len(), 2, "n1 tem 2 skills distintas (sem duplicar)");
        let backend = n1
            .iter()
            .find(|s| s.skill == "senior-backend")
            .expect("achou senior-backend");
        assert_eq!(
            backend.trigger.as_deref(),
            Some("query"),
            "a re-seleção atualiza o gatilho (último vence)"
        );
        assert_eq!(backend.source, "hub");
        assert_eq!(selected.skills_for("n2").len(), 1);
        assert!(selected.skills_for("ausente").is_empty());
    }

    /// Pureza preservada com as camadas novas: mesma entrada (projeções + foco) → byte-idêntico.
    #[test]
    fn build_stays_pure_with_contextual_layers() {
        let clues = ClueSet::from_records(&[clue_record(1, "s", &["p/"])]);
        let selected = SelectedSkills::from_records(&[skill_record(1, "n", "sk", None, "src")]);
        let a = briefing_with("s", &clues, "dev_app", "n", &selected);
        let b = briefing_with("s", &clues, "dev_app", "n", &selected);
        assert_eq!(a, b);
    }
}
