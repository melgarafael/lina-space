//! F1-0-6 critério 3 — consistência DOUTRINA×BINÁRIO: todo verbo `lina <x>` prometido
//! nos templates de doutrina (CLAUDE/AGENTS/GEMINI) existe no binário — **zero verbo
//! fantasma** (fecha a CLASSE do bug P2, não só `handoff`/`check`; era o fantasma que
//! fazia agentes contornarem com `ask --intent handoff`, seq 16242).
//!
//! F1-0-9 critério 1 — a doutrina contém a instrução de distribuição via CLAIM antes de
//! ask ad-hoc, com ≥2 exemplos concretos de fluxo `lina plan claim` (asserção
//! automatizada, não leitura manual).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Verbos de 1º nível do binário — espelho do `match` do `main` em `bin/lina.rs`.
/// Se um verbo novo entrar no bin, adicione-o AQUI e na doutrina (ou em nenhuma).
const BIN_VERBS: &[&str] = &[
    "whoami",
    "ask",
    "handoff",
    "check",
    "broadcast",
    "handshake",
    "plan",
    "guard",
    "resume",
    "do",
    "list",
    "vault",
    // F1-3-6: `lina spawn` (agente-cria-terminal com gate inforjável) — registrado no dispatch +
    // usage() do bin; a skill `lina-spawn-terminal` (próxima rodada) promete este verbo.
    "spawn",
    // F1-3-7: `lina retro` (auto-aprimoramento v0 — relatório determinístico, ZERO LLM; SÓ observa
    // e sugere, sem subcomando de mutação). Registrado no dispatch + usage(); a skill `lina-retro`
    // promete este verbo.
    "retro",
];

/// Sub-verbos do `lina plan` implementados (`run_plan`).
const PLAN_SUBVERBS: &[&str] = &["read", "claim", "check"];

fn doctrine_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/lina-doctrine")
}

fn templates() -> Vec<(String, String)> {
    ["CLAUDE.md", "AGENTS.md", "GEMINI.md"]
        .iter()
        .map(|f| {
            let p = doctrine_dir().join(f);
            (
                f.to_string(),
                std::fs::read_to_string(&p)
                    .unwrap_or_else(|e| panic!("ler template {}: {e}", p.display())),
            )
        })
        .collect()
}

/// Verbos `lina <x>` prometidos num texto (1º token após "`lina ").
fn promised_verbs(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (idx, _) in text.match_indices("`lina ") {
        let rest = &text[idx + "`lina ".len()..];
        let verb: String = rest
            .chars()
            .take_while(|c| c.is_ascii_lowercase() || *c == '-')
            .collect();
        if !verb.is_empty() {
            out.insert(verb);
        }
    }
    out
}

/// Sub-verbos prometidos como "`lina plan <sub>" num texto.
fn promised_plan_subverbs(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (idx, _) in text.match_indices("`lina plan ") {
        let rest = &text[idx + "`lina plan ".len()..];
        let sub: String = rest
            .chars()
            .take_while(|c| c.is_ascii_lowercase())
            .collect();
        if !sub.is_empty() {
            out.insert(sub);
        }
    }
    out
}

/// **Trava usage×main:** o usage do binário REAL anuncia todos os `BIN_VERBS` — se um
/// verbo entrar no `match` sem usage (ou vice-versa), este teste denuncia.
#[test]
fn usage_announces_every_bin_verb() {
    let exe = env!("CARGO_BIN_EXE_lina");
    let out = Command::new(exe).output().expect("rodar lina sem args");
    let usage = String::from_utf8_lossy(&out.stderr);
    for verb in BIN_VERBS {
        assert!(
            usage.contains(&format!("lina {verb}")),
            "usage não anuncia o verbo implementado `lina {verb}`"
        );
    }
}

/// **F1-0-6 critério 3 (a classe inteira):** todo verbo prometido em QUALQUER template
/// da doutrina existe no binário. Zero fantasma.
#[test]
fn doctrine_promises_only_verbs_that_exist() {
    let bin: BTreeSet<&str> = BIN_VERBS.iter().copied().collect();
    for (name, text) in templates() {
        let promised = promised_verbs(&text);
        let ghosts: Vec<&String> = promised
            .iter()
            .filter(|v| !bin.contains(v.as_str()))
            .collect();
        assert!(
            ghosts.is_empty(),
            "{name} promete verbos que o binário NÃO tem (fantasmas): {ghosts:?} — \
             implemente-os ou reescreva a doutrina para o mecanismo real"
        );
        // Os dois verbos da F1-0-6 seguem PROMETIDOS (a correção foi implementar, não apagar).
        assert!(
            promised.contains("handoff") && promised.contains("check"),
            "{name} deve seguir ensinando handoff e check (agora reais)"
        );
    }
}

/// Sub-verbos do plano: a doutrina só ensina o ciclo que o binário tem (read/claim/check).
#[test]
fn doctrine_plan_subverbs_exist_in_bin() {
    let subs: BTreeSet<&str> = PLAN_SUBVERBS.iter().copied().collect();
    for (name, text) in templates() {
        let promised = promised_plan_subverbs(&text);
        let ghosts: Vec<&String> = promised
            .iter()
            .filter(|v| !subs.contains(v.as_str()))
            .collect();
        assert!(
            ghosts.is_empty(),
            "{name} promete sub-verbos de `lina plan` inexistentes: {ghosts:?}"
        );
    }
}

/// SKILL.md instalados em `assets/lina-skills/` (varre o diretório — skills futuras da
/// F1-3 entram na trava automaticamente).
fn skills() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/lina-skills");
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return out;
    };
    for entry in rd.filter_map(Result::ok) {
        let p = entry.path().join("SKILL.md");
        if p.is_file() {
            let name = format!("{}/SKILL.md", entry.file_name().to_string_lossy());
            let text = std::fs::read_to_string(&p)
                .unwrap_or_else(|e| panic!("ler skill {}: {e}", p.display()));
            out.push((name, text));
        }
    }
    assert!(
        !out.is_empty(),
        "esperava >=1 SKILL.md em assets/lina-skills (lina-agent-bus)"
    );
    out
}

/// **Mini-lane pré-gate F1-0:** a trava de verbo fantasma vale para TODA promessa de
/// verbo no repo — também nos SKILL.md (a skill ensina os mesmos verbos da doutrina;
/// um fantasma aqui produz o MESMO contorno de campo que o critério 3 fecha).
#[test]
fn skills_promise_only_verbs_that_exist() {
    let bin: BTreeSet<&str> = BIN_VERBS.iter().copied().collect();
    let subs: BTreeSet<&str> = PLAN_SUBVERBS.iter().copied().collect();
    for (name, text) in skills() {
        let ghosts: Vec<String> = promised_verbs(&text)
            .into_iter()
            .filter(|v| !bin.contains(v.as_str()))
            .collect();
        assert!(
            ghosts.is_empty(),
            "{name} promete verbos que o binário NÃO tem (fantasmas): {ghosts:?}"
        );
        let sub_ghosts: Vec<String> = promised_plan_subverbs(&text)
            .into_iter()
            .filter(|v| !subs.contains(v.as_str()))
            .collect();
        assert!(
            sub_ghosts.is_empty(),
            "{name} promete sub-verbos de `lina plan` inexistentes: {sub_ghosts:?}"
        );
    }
}

/// **F1-0-9 critério 1:** os 3 templates contêm a instrução claim-ANTES-de-ask e ≥2
/// exemplos concretos de fluxo `lina plan claim` (a doutrina que empurra a coordenação
/// para o plano — baseline 14h: 0% de adoção).
#[test]
fn doctrine_teaches_claim_before_ask_with_examples() {
    for (name, text) in templates() {
        let lower = text.to_lowercase();
        assert!(
            lower.contains("claim antes de ask"),
            "{name} deve conter a regra de ouro 'claim ANTES de ask'"
        );
        let examples = text.matches("`lina plan claim T").count();
        assert!(
            examples >= 2,
            "{name} deve ter >=2 exemplos concretos de `lina plan claim T<id>` (tem {examples})"
        );
    }
}
