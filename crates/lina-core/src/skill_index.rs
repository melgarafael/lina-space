//! F3-5-4 · frente SKILLS (dono: Terminal J) — seletor DETERMINÍSTICO de skills.
//!
//! Seletor no core (ZERO LLM, Hermes C.1): um índice de skills (nome+gatilhos+tools-exigidas),
//! filtrado pelas tools/CLIs presentes no terminal, casando o gatilho com o contexto. É o
//! "RAG sem vetor": o gatilho É o retriever, a descrição É o embedding legível — a seleção é
//! recuperação barata, sem modelo.
//!
//! **ANTI-CICLO (`Cargo.toml` crava `bootstrap → core`):** o core NÃO importa `LINA_SKILLS`.
//! Aqui mora só a FUNÇÃO PURA sobre um índice RECEBIDO ([`select`] sobre [`SkillIndexEntry`]);
//! quem POPULA o índice a partir de `LINA_SKILLS` + `.claude/skills` é o caller em
//! `lina-bootstrap` (mesma doutrina de `briefing.rs`: fn pura + caller monta a entrada).

use std::collections::BTreeSet;

/// Uma entrada do índice de skills — o "anúncio" (nome + gatilhos + tools exigidas), nunca o
/// corpo (Hermes C.1: índice no prompt, corpo sob demanda). O caller em `lina-bootstrap`
/// POPULA o índice a partir de `LINA_SKILLS` + `.claude/skills`; o core jamais conhece o
/// catálogo (anti-ciclo).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillIndexEntry {
    /// Nome da skill (pasta/slug).
    pub name: String,
    /// Gatilhos de ativação (palavras/frases). Vazio = sem gatilho automático: a skill só
    /// entra por invocação explícita, nunca por match de contexto.
    pub triggers: Vec<String>,
    /// Ferramentas/CLIs que a skill EXIGE para rodar. Vazio = sem requisito (sempre capaz).
    pub requires: Vec<String>,
}

/// O resultado de uma seleção: a skill que casou + o gatilho específico (para `SkillSelected`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillSelection {
    /// Nome da skill selecionada.
    pub name: String,
    /// O gatilho que casou o contexto (sempre `Some` via [`select`]; `None` reservado para
    /// seleção por invocação explícita pelo caller).
    pub trigger: Option<String>,
}

/// As skills OFERECÍVEIS: filtra o índice pelas tools presentes — uma skill só aparece se TODAS
/// as tools que exige existem no terminal (Hermes C.1 `_skill_should_show`). Não oferecer
/// capacidade que o ambiente não roda evita ALUCINAR capacidade. `requires` vazio = sempre capaz.
#[must_use]
pub fn available<'a>(
    index: &'a [SkillIndexEntry],
    present_tools: &BTreeSet<String>,
) -> Vec<&'a SkillIndexEntry> {
    index
        .iter()
        .filter(|e| e.requires.iter().all(|t| present_tools.contains(t)))
        .collect()
}

/// As skills SELECIONADAS para `context`: das oferecíveis (tools presentes), as cujo gatilho
/// casa o contexto (substring case-insensitive — o gatilho É o retriever, "RAG sem vetor").
/// DETERMINÍSTICO, ZERO LLM. Preserva a ordem do índice (estável para replay).
#[must_use]
pub fn select(
    index: &[SkillIndexEntry],
    present_tools: &BTreeSet<String>,
    context: &str,
) -> Vec<SkillSelection> {
    let haystack = context.to_lowercase();
    available(index, present_tools)
        .into_iter()
        .filter_map(|e| {
            matched_trigger(e, &haystack).map(|trigger| SkillSelection {
                name: e.name.clone(),
                trigger: Some(trigger),
            })
        })
        .collect()
}

/// O 1º gatilho declarado da skill que aparece em `haystack_lower` (já minúsculo). `None` se
/// nenhum casa. Gatilho vazio nunca casa (não seleciona tudo por engano).
fn matched_trigger(entry: &SkillIndexEntry, haystack_lower: &str) -> Option<String> {
    entry
        .triggers
        .iter()
        .find(|t| !t.is_empty() && haystack_lower.contains(&t.to_lowercase()))
        .cloned()
}

/// Frontmatter neutro de uma skill (schema CLI-agnóstico, Hermes C.4): `name` obrigatório,
/// `description`/`trigger`/`requires` opcionais. SEM campos acoplados a provider
/// (`metadata.hermes.*`). `trigger`/`requires` aceitam CSV inline (`a, b`) — o formato que a
/// fábrica gera; skills legadas (só `name`+`description`) viram listas vazias.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SkillFrontmatter {
    pub name: String,
    pub description: String,
    pub triggers: Vec<String>,
    pub requires: Vec<String>,
}

/// Lê o frontmatter YAML-lite entre os dois `---` no topo do SKILL.md. Determinístico, sem dep
/// de YAML: linhas `chave: valor`; `trigger`/`requires` viram listas por split em vírgula. Uma
/// skill SEM frontmatter (ou sem os campos) vira entrada de triggers/requires vazios — entra no
/// índice como "sempre capaz, sem gatilho automático".
#[must_use]
pub fn parse_frontmatter(skill_md: &str) -> SkillFrontmatter {
    let mut fm = SkillFrontmatter::default();
    let Some(block) = frontmatter_block(skill_md) else {
        return fm;
    };
    for line in block.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "name" => fm.name = value.to_string(),
            "description" => fm.description = value.to_string(),
            "trigger" | "triggers" => fm.triggers = csv(value),
            "requires" | "requires_tools" => fm.requires = csv(value),
            _ => {}
        }
    }
    fm
}

/// O conteúdo entre o `---` de abertura (1ª linha) e o `---` de fechamento. `None` se não há
/// frontmatter delimitado. Tolera `\r\n` (Windows).
fn frontmatter_block(md: &str) -> Option<&str> {
    let rest = md
        .strip_prefix("---\n")
        .or_else(|| md.strip_prefix("---\r\n"))?;
    let end = rest.find("\n---")?;
    Some(&rest[..end])
}

/// CSV inline → itens trimados não-vazios (`"a, b ,"` → `["a", "b"]`).
fn csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tools(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    fn entry(name: &str, triggers: &[&str], requires: &[&str]) -> SkillIndexEntry {
        SkillIndexEntry {
            name: name.to_string(),
            triggers: triggers.iter().map(|s| (*s).to_string()).collect(),
            requires: requires.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    // ───────────── filtro por tools presentes (Hermes C.1) ─────────────

    /// CRITÉRIO DE ACEITE: skill cuja tool exigida NÃO está presente não aparece.
    #[test]
    fn available_hides_skill_missing_required_tool() {
        let index = [entry("playwright-e2e", &[], &["Bash", "Playwright"])];
        let got = available(&index, &tools(&["Bash", "Edit"]));
        assert!(got.is_empty(), "falta 'Playwright' → não oferece");
    }

    #[test]
    fn available_shows_skill_when_all_tools_present() {
        let index = [entry("playwright-e2e", &[], &["Bash", "Playwright"])];
        let got = available(&index, &tools(&["Bash", "Playwright", "Edit"]));
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "playwright-e2e");
    }

    #[test]
    fn available_skill_without_requires_is_always_offered() {
        let index = [entry("lina-code-doctrine", &[], &[])];
        assert_eq!(
            available(&index, &tools(&[])).len(),
            1,
            "sem requires = sempre capaz"
        );
    }

    // ───────────── seleção por gatilho (Hermes C.1) ─────────────

    /// CRITÉRIO DE ACEITE: gatilho que casa o contexto injeta a skill.
    #[test]
    fn select_matches_trigger_case_insensitive() {
        let index = [entry("lina-code-doctrine", &["conserta esse bug"], &[])];
        let got = select(&index, &tools(&[]), "Conserta esse BUG no login, por favor");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "lina-code-doctrine");
        assert_eq!(got[0].trigger.as_deref(), Some("conserta esse bug"));
    }

    /// O filtro de tools vence o gatilho: gatilho casa, mas a tool exigida falta → não seleciona.
    #[test]
    fn select_respects_tool_filter_even_when_trigger_matches() {
        let index = [entry(
            "playwright-e2e",
            &["rodar os testes e2e"],
            &["Playwright"],
        )];
        let got = select(&index, &tools(&["Bash"]), "quero rodar os testes e2e agora");
        assert!(
            got.is_empty(),
            "sem 'Playwright' não seleciona mesmo com gatilho casado"
        );
    }

    #[test]
    fn select_skips_skill_whose_trigger_is_absent() {
        let index = [entry("lina-design-doctrine", &["monta a landing"], &[])];
        let got = select(&index, &tools(&[]), "conserta esse bug no backend");
        assert!(got.is_empty(), "nenhum gatilho casa → nada selecionado");
    }

    #[test]
    fn select_without_triggers_never_matches_by_context() {
        let index = [entry("lina-agent-bus", &[], &[])];
        let got = select(&index, &tools(&[]), "qualquer contexto");
        assert!(
            got.is_empty(),
            "sem gatilho não casa por contexto (só invocação explícita)"
        );
    }

    #[test]
    fn select_preserves_index_order() {
        let index = [entry("a", &["foo"], &[]), entry("b", &["bar"], &[])];
        let got = select(&index, &tools(&[]), "tem foo e bar no texto");
        assert_eq!(
            got.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            vec!["a", "b"]
        );
    }

    // ───────────── parser de frontmatter neutro (Hermes C.4) ─────────────

    #[test]
    fn parse_frontmatter_reads_name_trigger_requires() {
        let md = "---\nname: deploy-helper\ndescription: ajuda a publicar\ntrigger: faz o deploy, publica a app\nrequires: Bash, Vercel\n---\n# corpo\n";
        let fm = parse_frontmatter(md);
        assert_eq!(fm.name, "deploy-helper");
        assert_eq!(fm.triggers, vec!["faz o deploy", "publica a app"]);
        assert_eq!(fm.requires, vec!["Bash", "Vercel"]);
    }

    /// Skill legada (formato Anthropic: só `name`+`description`) → triggers/requires vazios, mas
    /// `name` lido. Entra no índice como "sempre capaz, sem gatilho automático".
    #[test]
    fn parse_frontmatter_legacy_skill_has_empty_lists() {
        let md = "---\nname: lina-agent-bus\ndescription: comunicacao entre terminais\n---\nCorpo da skill.\n";
        let fm = parse_frontmatter(md);
        assert_eq!(fm.name, "lina-agent-bus");
        assert!(fm.triggers.is_empty());
        assert!(fm.requires.is_empty());
    }

    #[test]
    fn parse_frontmatter_without_block_is_default() {
        let fm = parse_frontmatter("sem frontmatter aqui\n");
        assert_eq!(fm, SkillFrontmatter::default());
    }
}
