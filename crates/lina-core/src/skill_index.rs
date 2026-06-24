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

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::events::{DomainEvent, EventRecord, SkillSignal};

/// Uma entrada do índice de skills — o "anúncio" (nome + descrição + gatilhos + tools exigidas),
/// nunca o corpo (Hermes C.1: índice no prompt, corpo sob demanda). O caller em `lina-bootstrap`
/// POPULA o índice a partir de `LINA_SKILLS` + `.claude/skills`; o core jamais conhece o
/// catálogo (anti-ciclo). `Serialize`/`Deserialize` (ADR 0045 C1): o índice é uma PROJEÇÃO
/// reconstruível persistível no disco — apagá-lo e reindexar a partir dos SKILL.md é sempre
/// válido (inv #4).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SkillIndexEntry {
    /// Nome da skill (pasta/slug).
    pub name: String,
    /// Descrição da skill (texto rico, orientado a gatilho) — o "documento" sobre o qual o
    /// retrieval léxico (ADR 0045 C1) pontua. `serde(default)`: índice antigo sem o campo
    /// reidrata (reconstruível).
    #[serde(default)]
    pub description: String,
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

// ───────────── ADR 0045 C1: retrieval léxico BM25 (top-k sob demanda) ─────────────
// O `select` acima casa gatilho por SUBSTRING — colapsa em escala (recência vence relevância,
// ADR 0045 §Contexto). `rank` é o retrieval que escala: pontua a QUERY contra o "documento" de
// cada skill (nome+descrição+gatilhos) por Okapi BM25 e devolve só os top-k candidatos — o
// mesmo padrão `ToolSearch` do CLI, aplicado a skills. Determinístico, ZERO LLM, 100% local.

/// Parâmetros Okapi BM25 canônicos: `k1` satura a frequência do termo; `b` normaliza pelo
/// tamanho do documento (descrições de skill variam de dezenas a ~1024 bytes — `b` evita que a
/// mais longa vença só por ser longa).
const BM25_K1: f64 = 1.2;
const BM25_B: f64 = 0.75;

/// Um candidato recuperado pelo retrieval (ADR 0045 C1): a skill + seu score BM25. O score é o
/// ponto onde o **C2** (ranking por outcome, R45-OUT) somará o histórico do event log — aqui é
/// só o sinal léxico.
#[derive(Debug, Clone, PartialEq)]
pub struct RankedSkill {
    /// Nome da skill recuperada.
    pub name: String,
    /// Score BM25 (>0; quanto maior, mais relevante à query).
    pub score: f64,
}

/// Recupera os **top-`k`** candidatos para `query` por BM25, DENTRE as oferecíveis (filtro de
/// tools — uma skill cuja tool falta nunca é candidata; o C0 já restringiu o disco por papel).
/// Skill sem nenhum termo da query casado é descartada (score 0 ≠ candidato). Empate de score
/// preserva a ordem do índice (sort estável → replay determinístico). ZERO LLM.
#[must_use]
pub fn rank(
    index: &[SkillIndexEntry],
    present_tools: &BTreeSet<String>,
    query: &str,
    k: usize,
) -> Vec<RankedSkill> {
    let candidates = available(index, present_tools);
    let docs: Vec<Vec<String>> = candidates.iter().map(|e| tokenize(&document(e))).collect();
    let query_terms = dedup(tokenize(query));
    if docs.is_empty() || query_terms.is_empty() {
        return Vec::new();
    }
    let n = docs.len() as f64;
    let total_len: usize = docs.iter().map(Vec::len).sum();
    let avgdl = if total_len == 0 {
        1.0
    } else {
        total_len as f64 / n
    };
    // df por termo da query (uma vez): em quantos documentos o termo aparece.
    let doc_freq: Vec<f64> = query_terms
        .iter()
        .map(|term| docs.iter().filter(|d| d.contains(term)).count() as f64)
        .collect();

    let mut scored: Vec<RankedSkill> = candidates
        .iter()
        .zip(&docs)
        .filter_map(|(entry, doc)| {
            let score = bm25_score(doc, avgdl, &query_terms, &doc_freq, n);
            (score > 0.0).then(|| RankedSkill {
                name: entry.name.clone(),
                score,
            })
        })
        .collect();
    // Maior score primeiro; `sort_by` é estável → empate mantém a ordem do índice.
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored.truncate(k);
    scored
}

/// Score BM25 de um documento `doc` contra os termos da query (com seus `doc_freq` pré-calculados
/// e a contagem de docs `n`). `avgdl` é o tamanho médio do corpus.
fn bm25_score(doc: &[String], avgdl: f64, query_terms: &[String], doc_freq: &[f64], n: f64) -> f64 {
    let dl = doc.len() as f64;
    query_terms
        .iter()
        .zip(doc_freq)
        .map(|(term, &df)| {
            let tf = doc.iter().filter(|t| *t == term).count() as f64;
            if tf == 0.0 {
                return 0.0;
            }
            // IDF da variante BM25+ (1+…): nunca negativo, mesmo p/ termo em >½ dos docs.
            let idf = (1.0 + (n - df + 0.5) / (df + 0.5)).ln();
            idf * (tf * (BM25_K1 + 1.0)) / (tf + BM25_K1 * (1.0 - BM25_B + BM25_B * dl / avgdl))
        })
        .sum()
}

/// O "documento" indexável de uma skill: nome + descrição + gatilhos, o texto sobre o qual o BM25
/// pontua. (As skills da safra Lina usam só `description`; gatilhos entram quando existem.)
fn document(entry: &SkillIndexEntry) -> String {
    format!(
        "{} {} {}",
        entry.name,
        entry.description,
        entry.triggers.join(" ")
    )
}

/// Tokeniza para o índice léxico: minúsculas, quebra em qualquer não-alfanumérico (Unicode —
/// preserva acentos do pt-br: "função" é um token). Vazios descartados.
fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Termos únicos preservando a 1ª ocorrência (uma palavra digitada 2× na query não domina o BM25).
fn dedup(tokens: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(tokens.len());
    for t in tokens {
        if seen.insert(t.clone()) {
            out.push(t);
        }
    }
    out
}

// ───────────── ADR 0045 C2/C3: task_kind, ligação determinística e ranking por outcome ─────────────
// C1 (rank) recupera por relevância léxica. C2 soma a MEMÓRIA DE RESULTADO (event log): a mesma
// skill, no mesmo `task_kind`, deu certo/errado antes? C3 desempata. O sinal é INFERIDO PASSIVAMENTE
// (a Lina nunca pergunta). Tudo determinístico (replay reconstrói idêntico) — ZERO LLM. Skill é
// DADO, JAMAIS autoridade: o outcome muda QUAL skill, nunca dispensa o gate de ação irreversível.

/// Stop-words pt/en (termos de função sem poder discriminante) — removidas do `task_kind` para a
/// chave agrupar por CONTEÚDO, não por preposição. Conjunto pequeno e deliberado (MVP).
const STOP_WORDS: &[&str] = &[
    "a", "o", "as", "os", "um", "uma", "uns", "de", "do", "da", "dos", "das", "e", "em", "no",
    "na", "nos", "nas", "para", "por", "com", "que", "ao", "aos", "se", "the", "of", "to", "for",
    "and", "in", "on", "an",
];

/// Normalização estável (spec 0045 §3.1 = ADR 0048 A2.a, costura 0045↔0048): lowercase + espaços
/// colapsados. A MESMA situação semântica nunca vira hashes diferentes por caixa/espaçamento.
/// (Definição idêntica a `mentality::canon`; mantida local seguindo a convenção do crate — cada
/// módulo de projeção tem seu helper de hash, cf. `approval`/`lifecycle`/`mentality`.)
fn canon(s: &str) -> String {
    s.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// `sha256_hex16(parts[0] \x1f parts[1] …)` — 8 bytes = 16 hex chars, separador `0x1f` evita
/// colisão por concatenação ambígua. Idêntico a `mentality::sha256_hex16` (mesma primitiva 0048).
fn sha256_hex16(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            hasher.update([0x1f]);
        }
        hasher.update(part.as_bytes());
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(16);
    for byte in digest.iter().take(8) {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// `task_kind` determinístico (spec 0045 §4; MESMA unidade do `situation_hash` do ADR 0048 — um
/// conceito, dois usos): `canon(role):t1-t2-t3`, onde t1..t3 são os termos dominantes da query
/// (sem stop-words), tomados por ocorrência (topicalidade) e depois ORDENADOS — a mesma intenção
/// em ordem diferente cai no mesmo `task_kind`. Reusa a tokenização do BM25 (C1).
#[must_use]
pub fn task_kind(role: &str, query: &str) -> String {
    let mut terms: Vec<String> = dedup(tokenize(query))
        .into_iter()
        .filter(|t| !STOP_WORDS.contains(&t.as_str()))
        .collect();
    terms.truncate(3); // top-3 por ocorrência (dominância)…
    terms.sort(); // …e ordenados para a chave ser canônica
    format!("{}:{}", canon(role), terms.join("-"))
}

/// `selection_id` determinístico (spec 0045 §3.1) — a chave que liga `SkillOutcome` ao
/// `SkillSelected`. `sel-` + sha256[..16] de `canon(node)|skill|task_kind|root_cause_id`. O
/// `root_cause_id` (id do turno) desambigua seleções repetidas; entra cru (é identificador).
/// Determinístico → replay reconstrói a MESMA ligação (não inventar id aleatório).
#[must_use]
pub fn selection_id(node: &str, skill: &str, task_kind: &str, root_cause_id: &str) -> String {
    let node = canon(node);
    let skill = canon(skill);
    let task_kind = canon(task_kind);
    format!(
        "sel-{}",
        sha256_hex16(&[
            node.as_str(),
            skill.as_str(),
            task_kind.as_str(),
            root_cause_id
        ])
    )
}

/// Peso do sinal no ranking: sucesso reforça (+1), as três falhas penalizam (−1). `Reworked`/
/// `HumanOverride` distinguem-se só para o painel; no score pesam igual a `Failure`.
fn signal_weight(signal: SkillSignal) -> f64 {
    match signal {
        SkillSignal::Success => 1.0,
        SkillSignal::Failure | SkillSignal::Reworked | SkillSignal::HumanOverride => -1.0,
    }
}

/// Score de OUTCOME por `(task_kind, skill)` reconstruído por REPLAY do log (ADR 0045 C2, padrão
/// `CostLedger`/`Mentality`): liga cada `SkillOutcome.selection_ref` ao `SkillSelected.selection_id`
/// e soma o peso do sinal. PURA sobre os records — reprocessar dá o MESMO score (inv #4). Eventos
/// indecodificáveis são pulados (projeção derivada, não validadora — padrão `briefing`/`mentality`).
#[must_use]
pub fn outcome_scores(records: &[EventRecord]) -> BTreeMap<(String, String), f64> {
    // 1ª passada: selection_id → (task_kind, skill).
    let mut by_selection: BTreeMap<String, (String, String)> = BTreeMap::new();
    for record in records {
        if let Ok(DomainEvent::SkillSelected {
            skill,
            task_kind,
            selection_id,
            ..
        }) = DomainEvent::from_record(&record.kind, record.version, record.payload.clone())
        {
            if !selection_id.is_empty() {
                by_selection.insert(selection_id, (task_kind, skill));
            }
        }
    }
    // 2ª passada: cada outcome soma no (task_kind, skill) da seleção que referencia.
    let mut scores: BTreeMap<(String, String), f64> = BTreeMap::new();
    for record in records {
        if let Ok(DomainEvent::SkillOutcome {
            selection_ref,
            signal,
            ..
        }) = DomainEvent::from_record(&record.kind, record.version, record.payload.clone())
        {
            if let Some(key) = by_selection.get(&selection_ref) {
                *scores.entry(key.clone()).or_insert(0.0) += signal_weight(signal);
            }
        }
    }
    scores
}

/// Peso do termo de outcome na combinação final (ADR 0045 C2). 1.0 = um histórico nítido vira a
/// escolha num quase-empate de retrieval; BM25 forte ainda manda quando a relevância é clara.
const OUTCOME_WEIGHT: f64 = 1.0;

/// Recupera o top-`k` combinando RETRIEVAL (BM25, C1) com o HISTÓRICO de outcome (C2) para o
/// `task_kind` desta query: `final = bm25 + OUTCOME_WEIGHT · outcome(task_kind, skill)`. `scores`
/// vem de [`outcome_scores`] (replay). Determinístico, ZERO LLM. **Skill é DADO, não autoridade:**
/// o score muda QUAL skill, jamais dispensa o gate (ADR 0045 §Segurança — provado por mutação).
#[must_use]
pub fn rank_with_outcome(
    index: &[SkillIndexEntry],
    present_tools: &BTreeSet<String>,
    query: &str,
    role: &str,
    k: usize,
    scores: &BTreeMap<(String, String), f64>,
) -> Vec<RankedSkill> {
    let tk = task_kind(role, query);
    // Todos os candidatos léxicos (k = MAX); o corte por k vem após somar o outcome.
    let mut ranked = rank(index, present_tools, query, usize::MAX);
    for r in &mut ranked {
        if let Some(outcome) = scores.get(&(tk.clone(), r.name.clone())) {
            r.score += OUTCOME_WEIGHT * outcome;
        }
    }
    ranked.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    ranked.truncate(k);
    ranked
}

/// Há EMPATE no topo? (top-2 com score igual) — o sinal do C3: empate **+ ação cara/irreversível**
/// → gate humano (decisão do CALLER, que conhece a ação); caso comum → o terminal decide e narra.
/// Esta fn só DETECTA o empate; a autoridade do gate permanece no caller (doutrina de segurança).
#[must_use]
pub fn is_top_tie(ranked: &[RankedSkill]) -> bool {
    matches!(ranked, [a, b, ..] if (a.score - b.score).abs() < f64::EPSILON)
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
/// de YAML: linhas `chave: valor`; `trigger`/`requires` viram listas por split em vírgula. Suporta
/// ESCALAR DOBRADO (`description: >-` seguido de linhas indentadas) — o formato real das skills da
/// safra Lina e do Anthropic; a continuação vira uma linha só (newlines → espaços). Uma skill SEM
/// frontmatter (ou sem os campos) vira entrada de triggers/requires vazios — entra no índice como
/// "sempre capaz, sem gatilho automático".
#[must_use]
pub fn parse_frontmatter(skill_md: &str) -> SkillFrontmatter {
    let mut fm = SkillFrontmatter::default();
    let Some(block) = frontmatter_block(skill_md) else {
        return fm;
    };
    let lines: Vec<&str> = block.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        i += 1;
        // Continuação de bloco dobrado já é consumida abaixo; aqui só chaves de topo (sem indento).
        if line.starts_with([' ', '\t']) {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        // `>-`/`>`/`|`/`|-` ou valor vazio = escalar dobrado: junta as linhas indentadas seguintes.
        let resolved = if is_fold_marker(value) {
            let mut folded = String::new();
            while i < lines.len() && lines[i].starts_with([' ', '\t']) {
                let cont = lines[i].trim();
                i += 1;
                if cont.is_empty() {
                    continue;
                }
                if !folded.is_empty() {
                    folded.push(' ');
                }
                folded.push_str(cont);
            }
            folded
        } else {
            value.to_string()
        };
        match key.trim() {
            "name" => fm.name = resolved,
            "description" => fm.description = resolved,
            "trigger" | "triggers" => fm.triggers = csv(&resolved),
            "requires" | "requires_tools" => fm.requires = csv(&resolved),
            _ => {}
        }
    }
    fm
}

/// Marcadores de escalar dobrado/literal YAML (`>`, `>-`, `|`, `|-`) ou valor inline ausente —
/// nos quais o valor real vem nas linhas de continuação indentadas.
fn is_fold_marker(value: &str) -> bool {
    matches!(value, ">" | ">-" | ">+" | "|" | "|-" | "|+" | "")
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
            description: String::new(),
            triggers: triggers.iter().map(|s| (*s).to_string()).collect(),
            requires: requires.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    fn described(name: &str, description: &str, requires: &[&str]) -> SkillIndexEntry {
        SkillIndexEntry {
            name: name.to_string(),
            description: description.to_string(),
            triggers: Vec::new(),
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

    /// CRITÉRIO DE ACEITE (formato real da safra Lina): `description: >-` dobrado em várias linhas
    /// indentadas vira UMA descrição (newlines→espaços), não o literal ">-". Sem isso o BM25
    /// indexaria lixo.
    #[test]
    fn parse_frontmatter_reads_folded_description() {
        let md = "---\nname: lina-code-doctrine\ndescription: >-\n  Anti-slop ao escrever código.\n  Use ao implementar ou corrigir um bug.\n---\n# corpo\n";
        let fm = parse_frontmatter(md);
        assert_eq!(fm.name, "lina-code-doctrine");
        assert_eq!(
            fm.description,
            "Anti-slop ao escrever código. Use ao implementar ou corrigir um bug."
        );
    }

    // ───────────── ADR 0045 C1: retrieval BM25 top-k ─────────────

    /// CRITÉRIO DE ACEITE (ADR 0045 §Verificação C1): com N skills de descrições controladas, a
    /// tarefa-alvo recupera a skill relevante no TOPO do top-k — não por substring, por relevância.
    #[test]
    fn rank_recovers_relevant_skill_at_top() {
        let index = [
            described(
                "lina-copy-doctrine",
                "escrever copy e landing de vendas",
                &[],
            ),
            described(
                "lina-code-doctrine",
                "anti-slop ao escrever ou corrigir código; conserta bug",
                &[],
            ),
            described(
                "lina-design-doctrine",
                "estilizar tela, fonte e paleta",
                &[],
            ),
        ];
        let got = rank(&index, &tools(&[]), "preciso consertar um bug no código", 5);
        assert_eq!(
            got[0].name, "lina-code-doctrine",
            "a mais relevante no topo"
        );
    }

    /// Top-k LIMITA o número de candidatos (o ponto do retrieval: poucos candidatos, não o acervo).
    #[test]
    fn rank_caps_results_at_k() {
        let index = [
            described("a", "deploy publicar build app", &[]),
            described("b", "deploy publicar release app", &[]),
            described("c", "deploy publicar app produção", &[]),
        ];
        let got = rank(&index, &tools(&[]), "deploy app", 2);
        assert_eq!(got.len(), 2, "k=2 corta o 3º candidato");
    }

    /// Skill sem NENHUM termo da query casado não é candidata (score 0 ≠ recuperada).
    #[test]
    fn rank_drops_skills_with_zero_match() {
        let index = [
            described("relevante", "montar a api de leads no servidor", &[]),
            described("irrelevante", "astrologia mapa astral signo", &[]),
        ];
        let got = rank(&index, &tools(&[]), "api de leads", 5);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "relevante");
    }

    /// O filtro de tools vence o retrieval: descrição casa, mas a tool exigida falta → fora dos
    /// candidatos (mesma doutrina do `select`: nunca oferecer capacidade que o ambiente não roda).
    #[test]
    fn rank_respects_tool_filter() {
        let index = [described(
            "playwright-e2e",
            "rodar testes e2e no navegador",
            &["Playwright"],
        )];
        let got = rank(&index, &tools(&["Bash"]), "rodar testes e2e", 5);
        assert!(
            got.is_empty(),
            "sem 'Playwright' não é candidata nem por BM25"
        );
    }

    /// Determinístico: a mesma query sobre o mesmo índice produz a mesma ordem (replay estável).
    #[test]
    fn rank_is_deterministic() {
        let index = [
            described("x", "deploy publicar app na vercel", &[]),
            described("y", "deploy publicar app no servidor", &[]),
        ];
        let a = rank(&index, &tools(&[]), "deploy app", 5);
        let b = rank(&index, &tools(&[]), "deploy app", 5);
        assert_eq!(a, b);
    }

    /// O índice é uma PROJEÇÃO serializável (ADR 0045 C1): round-trip JSON preserva a entrada —
    /// base do "índice no disco reconstruível".
    #[test]
    fn index_entry_json_round_trip() {
        let e = described("deploy-helper", "faz o deploy", &["Bash"]);
        let json = serde_json::to_string(&e).expect("serializa");
        let back: SkillIndexEntry = serde_json::from_str(&json).expect("desserializa");
        assert_eq!(e, back);
    }

    // ───────────── ADR 0045 C2/C3: task_kind, ligação, outcome e desempate ─────────────

    fn record(seq: u64, event: &DomainEvent) -> EventRecord {
        EventRecord {
            seq,
            ts: seq,
            kind: event.kind().to_string(),
            version: event.current_version(),
            payload: serde_json::to_value(event).expect("serializa"),
        }
    }

    fn sel_event(skill: &str, task_kind: &str, selection_id: &str) -> DomainEvent {
        DomainEvent::SkillSelected {
            node: "n".to_string(),
            skill: skill.to_string(),
            trigger: None,
            source: "catalog".to_string(),
            task_kind: task_kind.to_string(),
            candidates: Vec::new(),
            by_role: "BACKEND".to_string(),
            rank_reason: String::new(),
            selection_id: selection_id.to_string(),
        }
    }

    fn outcome_event(selection_ref: &str, signal: SkillSignal) -> DomainEvent {
        DomainEvent::SkillOutcome {
            selection_ref: selection_ref.to_string(),
            signal,
            evidence: String::new(),
        }
    }

    /// `task_kind` agrupa a MESMA intenção independente de ordem/caixa (mesma chave p/ o ranking).
    #[test]
    fn task_kind_groups_same_intent_regardless_of_order() {
        let a = task_kind("BACKEND", "validar api de leads");
        let b = task_kind("backend", "leads de api validar");
        assert_eq!(a, b, "ordem/caixa/stop-words não mudam o task_kind");
        assert!(a.starts_with("backend:"), "papel canonicalizado: {a}");
    }

    /// `selection_id` é determinístico (canon normaliza) e o `root_cause_id` desambigua repetições.
    #[test]
    fn selection_id_is_deterministic_and_disambiguates() {
        let a = selection_id("Node A", "skill", "backend:api", "rc1");
        let b = selection_id("node a", "skill", "backend:api", "rc1");
        assert_eq!(
            a, b,
            "canon(node) normaliza caixa → MESMA ligação no replay"
        );
        let c = selection_id("Node A", "skill", "backend:api", "rc2");
        assert_ne!(a, c, "root_cause_id diferente → seleção distinta");
        assert!(a.starts_with("sel-"));
    }

    /// CRITÉRIO DE ACEITE C2 (ADR 0045 §Verificação): `SkillOutcome{Failure}` p/ B + `{Success}` p/
    /// A, no mesmo task_kind, muda a escolha de B→A — mesmo B vencendo o empate léxico. NÃO é
    /// recência nem substring: é MEMÓRIA DE RESULTADO reconstruída por replay.
    #[test]
    fn outcome_changes_choice_over_lexical_winner() {
        // Nomes SEM hífen/letra solta (não colidem com termos da query). Descrições idênticas →
        // empate de BM25; `bravo` vem 1º no índice (vence o empate léxico).
        let index = [
            described("bravo", "montar api de leads no servidor", &[]),
            described("alpha", "montar api de leads no servidor", &[]),
        ];
        let role = "BACKEND";
        let query = "montar api de leads";

        // Controle: SEM histórico, o empate de BM25 cai na ordem do índice (bravo).
        let empty = BTreeMap::new();
        let lexical = rank_with_outcome(&index, &tools(&[]), query, role, 5, &empty);
        assert_eq!(
            lexical[0].name, "bravo",
            "controle: sem outcome vence o léxico"
        );

        // Com histórico: alpha teve Success, bravo teve Failure → alpha vence.
        let tk = task_kind(role, query);
        let sa = selection_id("n-alpha", "alpha", &tk, "rc1");
        let sb = selection_id("n-bravo", "bravo", &tk, "rc1");
        let log = [
            record(1, &sel_event("alpha", &tk, &sa)),
            record(2, &sel_event("bravo", &tk, &sb)),
            record(3, &outcome_event(&sb, SkillSignal::Failure)),
            record(4, &outcome_event(&sa, SkillSignal::Success)),
        ];
        let scores = outcome_scores(&log);
        let ranked = rank_with_outcome(&index, &tools(&[]), query, role, 5, &scores);
        assert_eq!(
            ranked[0].name, "alpha",
            "alpha venceu bravo pelo HISTÓRICO de resultado, não por relevância léxica"
        );
    }

    /// A projeção de score é reconstruída por REPLAY e é determinística (mesmo log → mesmo score).
    #[test]
    fn outcome_scores_replay_is_deterministic() {
        let tk = "backend:api".to_string();
        let sa = selection_id("n", "skill-a", &tk, "rc1");
        let log = [
            record(1, &sel_event("skill-a", &tk, &sa)),
            record(2, &outcome_event(&sa, SkillSignal::Success)),
            record(3, &outcome_event(&sa, SkillSignal::Reworked)),
        ];
        let first = outcome_scores(&log);
        assert_eq!(first, outcome_scores(&log), "replay reconstrói idêntico");
        // Success(+1) + Reworked(−1) = 0.0 no (task_kind, skill).
        assert_eq!(*first.get(&(tk, "skill-a".to_string())).unwrap(), 0.0);
    }

    /// `SkillOutcome` cujo `selection_ref` não casa nenhum `SkillSelected` é ignorado (sem score
    /// órfão) — a ligação é estrita.
    #[test]
    fn orphan_outcome_is_ignored() {
        let log = [record(
            1,
            &outcome_event("sel-inexistente", SkillSignal::Success),
        )];
        assert!(outcome_scores(&log).is_empty());
    }

    /// CRITÉRIO DE ACEITE (aditividade, ADR 0045 §6): um `SkillSelected` ANTIGO (só os 4 campos)
    /// reidrata sem erro — `serde(default)` preenche os novos. Replay de log antigo nunca quebra.
    #[test]
    fn old_skill_selected_rehydrates_with_defaults() {
        let full = sel_event("s", "backend:api", "sel-x");
        let mut payload = serde_json::to_value(&full).expect("serializa");
        let obj = payload.as_object_mut().expect("objeto");
        for novo in [
            "task_kind",
            "candidates",
            "by_role",
            "rank_reason",
            "selection_id",
        ] {
            obj.remove(novo);
        }
        let ev = DomainEvent::from_record("SkillSelected", full.current_version(), payload)
            .expect("log antigo reidrata sem erro");
        match ev {
            DomainEvent::SkillSelected {
                task_kind,
                candidates,
                selection_id,
                ..
            } => {
                assert_eq!(task_kind, "");
                assert!(candidates.is_empty());
                assert_eq!(selection_id, "");
            }
            other => panic!("variante errada: {other:?}"),
        }
    }

    /// C3: empate no topo é detectado (sinal p/ gate humano em ação cara); diferença clara não.
    #[test]
    fn is_top_tie_detects_equal_top_scores() {
        let tie = [
            RankedSkill {
                name: "a".to_string(),
                score: 2.0,
            },
            RankedSkill {
                name: "b".to_string(),
                score: 2.0,
            },
        ];
        assert!(is_top_tie(&tie));
        let clear = [
            RankedSkill {
                name: "a".to_string(),
                score: 3.0,
            },
            RankedSkill {
                name: "b".to_string(),
                score: 1.0,
            },
        ];
        assert!(!is_top_tie(&clear));
        assert!(!is_top_tie(&tie[..1]), "1 candidato não é empate");
    }
}
