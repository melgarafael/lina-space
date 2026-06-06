//! `role_suggester` — **contrato de fronteira F1-2-2 ↔ F1-2-3** (sub-entregável coordenado).
//!
//! O ciclo entre o modal (F1-2-2, consome) e o co-piloto de papel (F1-2-3, implementa) é quebrado
//! por ESTE trait: **nome → sugestão + explicação**. O modal só conhece [`RoleSuggester`]; quem
//! está por trás pode evoluir (heurística W3-1 hoje; biblioteca de papéis + templates na F1-2-3)
//! sem tocar o modal.
//!
//! ## Contrato (estável — mudanças aqui são de fronteira, coordenar via Maestro)
//! - `suggest(name)` é **puro, local, determinístico e barato** (chamável a cada debounce de
//!   digitação; sem I/O, sem rede, sem LLM — invariantes #1/#2).
//! - `None` = "nada confiante a sugerir" → a UI **silencia** a linha-fantasma (M6-E2: o co-piloto
//!   indisponível/incerto nunca vira popup nem bloqueia a criação).
//! - `Some` carrega [`RoleSuggestion`]: o papel **canônico** (contrato do roster/`VIBE_ROLE`),
//!   o rótulo **leigo** pt-br, 1 frase do que o papel faz e o **porquê** da sugestão (13.3 rec.
//!   #8: "sempre mostrar por que o agente sugeriu") — tudo pronto para a tela, sem jargão.
//! - Sugestão **nunca** é aplicada silenciosamente (F1-2-3 critério 4): aplicar é gesto do
//!   usuário (Tab/clique) — responsabilidade do consumidor.
//!
//! A implementação inicial ([`W31Suggester`]) adapta a heurística **papel-pelo-nome da W3-1**
//! (`lina-role-discovery`), reusando o MESMO registry (com o overlay app-side `reviewer`) que o
//! `bridge::create_agent` usa para gravar o papel — sugestão e atribuição nunca divergem.

use std::sync::OnceLock;

use lina_role_discovery::{MatchStrategy, RoleRegistry};

// ═══════════════════════════ o contrato ═══════════════════════════

/// Uma sugestão de papel pronta para a tela (copy leiga; o `role` canônico fica por baixo).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleSuggestion {
    /// Papel **canônico** do roster (ex.: `"reviewer"`, `"BACKEND"`) — o que vira `VIBE_ROLE`/
    /// `NodeRoleAssigned`. Nunca mostrado cru ao leigo.
    pub role: String,
    /// Rótulo humano pt-br (ex.: `"Revisor"`) — o que aparece na linha-fantasma e no cartão.
    pub label: String,
    /// 1 frase leiga "o que ele faz" (cartão de Papel do M6).
    pub blurb: String,
    /// Por que sugeri (tooltip ⓘ — transparência, 13.3 rec. #8). Leigo, sem regex/jargão.
    pub why: String,
}

/// **O contrato de fronteira**: nome digitado → sugestão de papel (ou nada confiante).
pub trait RoleSuggester: Send + Sync {
    fn suggest(&self, name: &str) -> Option<RoleSuggestion>;
}

// ═══════════════════════════ registry compartilhado (W3-1 + overlay do app) ═══════════════════════════

/// Papéis que o registry default (lina-role-discovery) não cobre, mas o shell precisa.
/// (Movido do `bridge` na F1-2-2 — fonte única: sugestão e atribuição usam o MESMO registry.)
const APP_ROLE_OVERLAY: &str = "\
roles:
  - role: reviewer
    patterns: [revisor, reviewer, review, revisao]
    skills: [pr-review-toolkit:review-pr]
";

/// Registry de papéis (W3-1), construído UMA vez (regexes compiladas) e cacheado. `None` se o
/// YAML embutido falhar (não deve ocorrer) → o caller cai num papel genérico, nunca panica.
pub fn role_registry() -> Option<&'static RoleRegistry> {
    static REG: OnceLock<Option<RoleRegistry>> = OnceLock::new();
    REG.get_or_init(|| {
        let mut reg = RoleRegistry::with_defaults()
            .map_err(|e| {
                eprintln!("lina-gpui: role-registry default inválido ({e}); papel genérico")
            })
            .ok()?;
        if let Err(e) = reg.overlay_yaml_str(APP_ROLE_OVERLAY, "<app-role-overlay>") {
            eprintln!(
                "lina-gpui: overlay de papel 'reviewer' falhou ({e}); seguindo com o default"
            );
        }
        Some(reg)
    })
    .as_ref()
}

// ═══════════════════════════ implementação inicial (adapta a W3-1) ═══════════════════════════

/// Sugestor inicial: a heurística determinística papel-pelo-nome da W3-1. A F1-2-3 o substitui/
/// enriquece (biblioteca de papéis, templates) por trás do MESMO trait.
#[derive(Debug, Clone, Copy, Default)]
pub struct W31Suggester;

impl RoleSuggester for W31Suggester {
    fn suggest(&self, name: &str) -> Option<RoleSuggestion> {
        let name = name.trim();
        if name.len() < 3 {
            return None; // M6-E2: o co-piloto dispara com 3+ caracteres.
        }
        let reg = role_registry()?;
        let a = reg.infer_role(name);
        // Incerto (numérico/genérico/fallback — `needs_confirmation`) = NADA confiante a sugerir:
        // a linha-fantasma silencia (nunca "Parece um Desenvolvedor" para "Banana").
        if a.needs_confirmation {
            return None;
        }
        let (label, blurb) = humanize(&a.role);
        Some(RoleSuggestion {
            why: why_for(name, &label, a.matched_by),
            role: a.role,
            label,
            blurb,
        })
    }
}

/// Papel canônico → (rótulo leigo, 1 frase "o que ele faz"). Papéis fora da tabela ganham um
/// rótulo legível (Title Case do canônico) — nunca jargão cru tipo `LLM_ENGINEER` na tela.
#[must_use]
pub fn humanize(role: &str) -> (String, String) {
    let (label, blurb) = match role {
        "reviewer" => (
            "Revisor",
            "Revisa o trabalho dos colegas e aponta o que melhorar.",
        ),
        "DEVELOPER" => (
            "Desenvolvedor",
            "Constrói o que for preciso e delega o que for de outro papel.",
        ),
        "MAESTRO" => (
            "Maestro",
            "Coordena o time: distribui tarefas e cobra os resultados.",
        ),
        "ARQUITETO" => (
            "Arquiteto",
            "Desenha a estrutura do projeto antes de construir.",
        ),
        "FRONTEND" => (
            "Especialista em telas",
            "Cuida do que você vê: telas, botões e interações.",
        ),
        "BACKEND" => (
            "Especialista em bastidores",
            "Cuida do motor por trás: dados, regras e integrações.",
        ),
        "LLM_ENGINEER" => (
            "Especialista em IA",
            "Ajusta como os assistentes de IA pensam e respondem.",
        ),
        "DATA_ENGINEER" => (
            "Especialista em dados",
            "Organiza e prepara os dados do seu negócio.",
        ),
        "UIUX_DESIGNER" => ("Designer", "Desenha a aparência e a experiência de uso."),
        "WRITER" => ("Escritor", "Escreve textos, posts e roteiros no seu tom."),
        "QA" => ("Testador", "Valida e testa o que os colegas entregam."),
        "BUG_FIXER" => ("Caça-problemas", "Investiga e conserta o que quebrou."),
        "CURADOR" => ("Curador", "Garimpa e organiza referências e conteúdo."),
        "AUTOMATOR" => (
            "Automatizador",
            "Cria rotinas que trabalham sozinhas para você.",
        ),
        other => {
            // Title Case do canônico (`DATA_X` → `Data x`) — legível, nunca SNAKE_CASE na tela.
            let mut label = other.to_lowercase().replace('_', " ");
            if let Some(first) = label.get_mut(0..1) {
                first.make_ascii_uppercase();
            }
            return (label, "Colabora no time conforme o papel.".to_string());
        }
    };
    (label.to_string(), blurb.to_string())
}

/// O "por quê" leigo da sugestão, derivado da estratégia que casou (transparência sem jargão).
fn why_for(name: &str, label: &str, matched_by: MatchStrategy) -> String {
    match matched_by {
        MatchStrategy::ExplicitOverride => {
            format!("Você indicou o papel entre colchetes no nome \u{201c}{name}\u{201d}.")
        }
        MatchStrategy::ExactName | MatchStrategy::AtPrefix => {
            format!("O nome \u{201c}{name}\u{201d} é como esse papel costuma ser chamado.")
        }
        MatchStrategy::Keyword => {
            format!("O nome \u{201c}{name}\u{201d} contém uma palavra típica de {label}.")
        }
        // `needs_confirmation` já devolveu `None` antes; estes braços existem por exaustividade.
        MatchStrategy::GenericNumeric | MatchStrategy::DefaultFallback => {
            "Palpite genérico — confirme você mesmo.".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// O caminho do critério de aceite: "Revisor" → papel canônico `reviewer`, com rótulo leigo
    /// e o porquê presente (13.3 rec. #8). Mesmo registry da atribuição (overlay incluso).
    #[test]
    fn revisor_suggests_reviewer_with_why() {
        let s = W31Suggester.suggest("Revisor").expect("sugestão");
        assert_eq!(s.role, "reviewer");
        assert_eq!(s.label, "Revisor");
        assert!(
            !s.blurb.is_empty(),
            "cartão precisa da frase 'o que ele faz'"
        );
        assert!(s.why.contains("Revisor"), "porquê cita o nome: {}", s.why);
    }

    /// Nomes distintos → sugestões distintas e coerentes (espinha do critério 1 da F1-2-3).
    #[test]
    fn distinct_names_yield_distinct_roles() {
        let role = |n: &str| W31Suggester.suggest(n).map(|s| s.role);
        assert_eq!(role("Revisor").as_deref(), Some("reviewer"));
        assert_eq!(role("@Dev Backend").as_deref(), Some("BACKEND"));
        assert_eq!(
            role("Designer de Landing").as_deref(),
            Some("UIUX_DESIGNER")
        );
    }

    /// Incerto (genérico/numérico/fallback) → `None`: a linha-fantasma SILENCIA — nunca
    /// "Parece um Desenvolvedor" para um nome sem sinal (anti-falsa-confiança).
    #[test]
    fn uncertain_names_suggest_nothing() {
        assert_eq!(W31Suggester.suggest("Banana"), None, "fallback não sugere");
        assert_eq!(
            W31Suggester.suggest("terminal 3"),
            None,
            "numérico não sugere"
        );
        assert_eq!(
            W31Suggester.suggest("ab"),
            None,
            "menos de 3 chars não dispara"
        );
        assert_eq!(W31Suggester.suggest("   "), None);
    }

    /// O rótulo na tela é SEMPRE leigo: nenhum papel canônico em SNAKE_CASE/inglês cru vaza
    /// (critério 2 da F1-2-2: zero jargão em qualquer estado do modal).
    #[test]
    fn labels_are_lay_never_canonical_jargon() {
        let reg = role_registry().expect("registry");
        for role in reg.role_names() {
            let (label, blurb) = humanize(role);
            assert!(
                !label.contains('_'),
                "underscore no rótulo de {role}: {label}"
            );
            assert_ne!(
                label,
                label.to_uppercase(),
                "rótulo todo-maiúsculo (canônico cru) p/ {role}: {label}"
            );
            assert!(!blurb.is_empty());
        }
        // papéis desconhecidos (futuros) também saem legíveis.
        let (label, _) = humanize("GROWTH_HACKER");
        assert_eq!(label, "Growth hacker");
    }
}
