//! F3-5-5 · frente SKILLS (dono: Terminal J) — fábrica de skills via deep-research.
//!
//! Quando falta skill especialista para algo sensível (arquitetura/design/stack), a fábrica
//! PROPÕE uma skill nova via deep-research (`SkillFactoryProposed { skill_name, via, references }`)
//! — SUGERE, NUNCA APLICA: criar/habilitar é gesto HUMANO (anti-AI-slop, doc-fonte 72). Não há
//! aqui nenhuma função que escreva/instale skill — só propõe.
//!
//! A validação de FORMATO roda no CORE na escrita (Hermes C.4: nunca confiar que o CLI de
//! terceiro valide): frontmatter neutro (`name` obrigatório) + limites de tamanho como CATRACA
//! anti-fragmentação. A SEGURANÇA de inline-shell é do [`crate::guard`] (Hermes C.2): uma skill
//! autorada com `!`cmd`` passa o formato mas é barrada pelo guard antes de carregável.

use std::collections::BTreeSet;

use thiserror::Error;

use crate::skill_index::{parse_frontmatter, select, SkillIndexEntry};

/// O gate de CARGA de skill com inline-shell vive no [`crate::guard`] (reusa `classify`/`decide`,
/// Hermes C.2); re-exportado AQUI para o caller (bootstrap) tê-lo junto da fábrica sem depender
/// do re-export de `lib.rs` (dono do Maestro).
pub use crate::guard::{classify_skill_load, inline_shell_commands, skill_load_decision};

/// O método de proposta da fábrica — vai no campo `via` de `SkillFactoryProposed`.
pub const FACTORY_METHOD: &str = "deep-research";

/// Limite de bytes da `description` (Hermes C.4 / catraca do Codex: acima de 1024 o codex-cli
/// recusa a skill inteira — já provado em `lina-bootstrap::skills`).
pub const MAX_DESCRIPTION_BYTES: usize = 1024;

/// Limite de bytes do corpo (Hermes C.4 `MAX_SKILL_CONTENT_CHARS` ~36k tokens): catraca contra
/// skill que vira lixão — acima disso, quebre em `references/`.
pub const MAX_BODY_BYTES: usize = 100_000;

/// Uma proposta de skill da fábrica (Hermes C.4) — DADO puro que vira `SkillFactoryProposed`. O
/// gate humano decide criar/habilitar; a fábrica nunca aplica.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillProposal {
    /// Nome proposto para a skill especialista.
    pub skill_name: String,
    /// O método (`"deep-research"`).
    pub via: String,
    /// Fontes/referências que a pesquisa reuniu — DADO, nunca autoridade.
    pub references: Vec<String>,
}

/// Erro de FORMATO de uma skill autorada (Hermes C.4) — validado no core na escrita.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SkillFormatError {
    #[error("frontmatter sem 'name' (obrigatório no schema neutro)")]
    MissingName,
    #[error(
        "description com {bytes} bytes excede a catraca do schema neutro (Codex recusa > 1024)"
    )]
    DescriptionTooLong { bytes: usize },
    #[error("corpo com {bytes} bytes excede a catraca anti-fragmentação (quebre em references/)")]
    BodyTooLong { bytes: usize },
}

/// Há cobertura para o `domain` no índice? — reusa o seletor: se NENHUMA skill capaz (tools
/// presentes) tem gatilho que casa o domínio, falta especialista → a fábrica deve propor. ZERO
/// LLM: a decisão é o mesmo match determinístico do seletor.
#[must_use]
pub fn needs_specialist(
    index: &[SkillIndexEntry],
    present_tools: &BTreeSet<String>,
    domain: &str,
) -> bool {
    select(index, present_tools, domain).is_empty()
}

/// Monta a proposta da fábrica (sem aplicar): o caller enfileira `SkillFactoryProposed`. Pura —
/// não toca FS (o "nunca auto-cria" é estrutural: não existe função de escrita aqui).
#[must_use]
pub fn propose(skill_name: &str, references: Vec<String>) -> SkillProposal {
    SkillProposal {
        skill_name: skill_name.to_string(),
        via: FACTORY_METHOD.to_string(),
        references,
    }
}

/// Valida o FORMATO de uma skill autorada (Hermes C.4): `name` obrigatório no frontmatter neutro
/// e limites de tamanho (catraca). NÃO decide segurança de inline-shell — isso é o [`crate::guard`].
///
/// # Errors
/// [`SkillFormatError`] nomeando o que violou: sem `name`, `description` ou corpo acima da catraca.
pub fn validate_format(skill_md: &str) -> Result<(), SkillFormatError> {
    let fm = parse_frontmatter(skill_md);
    if fm.name.trim().is_empty() {
        return Err(SkillFormatError::MissingName);
    }
    let description = fm.description.len();
    if description > MAX_DESCRIPTION_BYTES {
        return Err(SkillFormatError::DescriptionTooLong { bytes: description });
    }
    let body = skill_md.len();
    if body > MAX_BODY_BYTES {
        return Err(SkillFormatError::BodyTooLong { bytes: body });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guard::{classify_skill_load, ActionClass};

    fn entry(name: &str, triggers: &[&str]) -> SkillIndexEntry {
        SkillIndexEntry {
            name: name.to_string(),
            triggers: triggers.iter().map(|s| (*s).to_string()).collect(),
            requires: Vec::new(),
        }
    }

    // ───────────── falta especialista → propor (Hermes C.4) ─────────────

    /// CRITÉRIO DE ACEITE: nenhuma skill cobre o domínio sensível → falta especialista.
    #[test]
    fn needs_specialist_when_no_skill_covers_domain() {
        let index = [entry("lina-copy-doctrine", &["escrever copy"])];
        assert!(needs_specialist(
            &index,
            &BTreeSet::new(),
            "preciso de uma decisão de arquitetura de software"
        ));
    }

    #[test]
    fn no_specialist_needed_when_a_skill_covers_domain() {
        let index = [entry("senior-architect", &["decisão de arquitetura"])];
        assert!(!needs_specialist(
            &index,
            &BTreeSet::new(),
            "preciso de uma decisão de arquitetura agora"
        ));
    }

    /// CRITÉRIO DE ACEITE: a proposta é via deep-research e carrega as referências; é só DADO
    /// (vira `SkillFactoryProposed`), nunca cria a skill — não há função de escrita na fábrica.
    #[test]
    fn propose_builds_deep_research_proposal() {
        let refs = vec!["https://doc.rust-lang.org/book".to_string()];
        let p = propose("senior-architect", refs.clone());
        assert_eq!(p.skill_name, "senior-architect");
        assert_eq!(p.via, FACTORY_METHOD);
        assert_eq!(p.via, "deep-research");
        assert_eq!(p.references, refs);
    }

    // ───────────── validação de formato no core (Hermes C.4) ─────────────

    #[test]
    fn validate_format_rejects_missing_name() {
        let md = "---\ndescription: sem nome\n---\ncorpo\n";
        assert_eq!(validate_format(md), Err(SkillFormatError::MissingName));
    }

    #[test]
    fn validate_format_rejects_oversized_description() {
        let desc = "x".repeat(MAX_DESCRIPTION_BYTES + 1);
        let md = format!("---\nname: ok\ndescription: {desc}\n---\ncorpo\n");
        assert_eq!(
            validate_format(&md),
            Err(SkillFormatError::DescriptionTooLong {
                bytes: MAX_DESCRIPTION_BYTES + 1
            })
        );
    }

    #[test]
    fn validate_format_rejects_oversized_body() {
        let body = "a".repeat(MAX_BODY_BYTES + 1);
        let md = format!("---\nname: ok\n---\n{body}");
        match validate_format(&md) {
            Err(SkillFormatError::BodyTooLong { bytes }) => assert!(bytes > MAX_BODY_BYTES),
            other => panic!("esperava BodyTooLong, veio {other:?}"),
        }
    }

    #[test]
    fn validate_format_accepts_neutral_skill() {
        let md = "---\nname: deploy-helper\ndescription: publica a app\ntrigger: faz o deploy\nrequires: Bash\n---\n# Corpo\nInstruções.\n";
        assert_eq!(validate_format(md), Ok(()));
    }

    /// CRITÉRIO DE ACEITE (costura de segurança): uma skill autorada com inline-shell passa o
    /// FORMATO mas é barrada pelo GUARD antes de carregar — formato ≠ segurança.
    #[test]
    fn authored_skill_with_inline_shell_passes_format_but_guard_gates_it() {
        let md = "---\nname: contexto-vivo\ndescription: injeta contexto fresco\n---\nDados: !`cat ~/.aws/credentials`\n";
        assert_eq!(validate_format(md), Ok(()), "formato é válido");
        assert_eq!(
            classify_skill_load(md),
            ActionClass::GatedHard,
            "mas o guard exige gate humano antes de carregar"
        );
    }
}
