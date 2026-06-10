//! `lina-role-discovery` — descoberta de **papel pelo nome do terminal** (story **W3-1**).
//!
//! ## Âncora de continuidade (ver `CLAUDE.md`)
//! Invariante **"Sem LLM/harness/chat próprios"**: o papel de um terminal é
//! inferido **deterministicamente e em puro Rust**, jamais por um modelo. Um
//! leigo abre o canvas, dá um nome ao terminal (`@Arquiteto`, `Dev Backend`,
//! `terminal 3`) e o sistema já sabe — no turno 0, sem rede, sem config manual —
//! qual o papel, quais skills carregar e se a atribuição é confiável o bastante
//! para auto-orquestrar ou se pede confirmação humana.
//!
//! ## Fonte da verdade
//! O registry default ([`RoleRegistry::with_defaults`]) é o YAML embutido
//! `default-roles.yaml`, transcrito do vault `21 - Camada de IA e Orquestracao`
//! §2.2 (que estende `maestri-orchestrator/references/role-mapping.md`). O
//! formato é declarativo: trocar/estender papéis = editar YAML, sem recompilar
//! ([`RoleRegistry::from_yaml_file`] / [`RoleRegistry::overlay_yaml_str`]).
//!
//! ## Como [`RoleRegistry::infer_role`] resolve (cascata, primeiro acerto vence)
//! As 5 estratégias da story, mais o **override explícito** que o doc §2.1 manda
//! "sempre vencer". Rigor decrescente — é a cascata que torna o resultado
//! determinístico mesmo quando um nome carrega palavras de papéis diferentes:
//!
//! | # | [`MatchStrategy`] | Quando | Exemplo |
//! |---|---|---|---|
//! | 0 | [`ExplicitOverride`](MatchStrategy::ExplicitOverride) | `[PAPEL]` em colchetes casa um papel canônico | `"Pesquisa [ARQUITETO]"` → ARQUITETO |
//! | 1 | [`ExactName`](MatchStrategy::ExactName) | nome (sem `@`) casa um pattern por inteiro | `"@Arquiteto"`, `"Arquiteto"`, `"ui ux"` |
//! | 2 | [`AtPrefix`](MatchStrategy::AtPrefix) | nome começa com `@` e o pattern é prefixo (o mais longo vence) | `"@Dev Backend"` → BACKEND |
//! | 3 | [`Keyword`](MatchStrategy::Keyword) | um pattern aparece como início de palavra | `"build the backend api"` → BACKEND |
//! | 4 | [`GenericNumeric`](MatchStrategy::GenericNumeric) | nome genérico `"terminal N"`/número | `"terminal 3"` → DEVELOPER (pede confirmação) |
//! | 5 | [`DefaultFallback`](MatchStrategy::DefaultFallback) | nada casou | `"Banana"` → DEVELOPER (pede confirmação) |
//!
//! Garantia central (gate de aceite): `"@Arquiteto"` e `"Arquiteto"` resolvem
//! **ARQUITETO, nunca MAESTRO** — MAESTRO só sai por seus próprios patterns
//! (`@Maestro|orquest|coord|lider|master`), jamais como fallback.

#![forbid(unsafe_code)]

use std::path::Path;

use regex::Regex;
use serde::Deserialize;
use thiserror::Error;

/// Registry default embutido — descoberta sem I/O nem rede em produção.
const DEFAULT_REGISTRY_YAML: &str = include_str!("default-roles.yaml");

/// Qual estratégia da cascata resolveu o papel. Exposto para transparência e
/// telemetria (e para os testes provarem "um teste por estratégia").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchStrategy {
    /// `[PAPEL]` explícito em colchetes (doc §2.1: "sempre vence").
    ExplicitOverride,
    /// O nome (sem `@`) casa um pattern por inteiro.
    ExactName,
    /// Nome com prefixo `@`: o pattern é prefixo do nome (o mais longo vence).
    AtPrefix,
    /// Um pattern aparece como início de palavra em qualquer ponto do nome.
    Keyword,
    /// Nome genérico do tipo `"terminal N"` ou só um número.
    GenericNumeric,
    /// Nada casou — fallback canônico.
    DefaultFallback,
}

/// Resultado da inferência: papel canônico, skills a carregar, se a atribuição
/// pede confirmação humana antes de auto-orquestrar, e qual estratégia decidiu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleAssignment {
    /// Papel canônico (ex.: `"ARQUITETO"`, `"DEVELOPER"`).
    pub role: String,
    /// Skills primárias a carregar para esse papel.
    pub skills: Vec<String>,
    /// `true` quando o papel é incerto (numérico/genérico/fallback): a postura
    /// "assistido" do doc §6 exige confirmação do Maestro antes de delegar.
    pub needs_confirmation: bool,
    /// Estratégia da cascata que produziu este resultado.
    pub matched_by: MatchStrategy,
}

// ───────────────────────── Schema YAML (deserialização) ─────────────────────

/// Registry completo lido do YAML.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistrySpec {
    fallback: FallbackSpec,
    roles: Vec<RoleSpec>,
}

/// Entrada de papel: nome canônico + patterns (regex) + skills + flag.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoleSpec {
    role: String,
    patterns: Vec<String>,
    skills: Vec<String>,
    #[serde(default)]
    needs_confirmation: bool,
}

/// Papel de fallback (sem patterns; usado quando nada casa).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct FallbackSpec {
    role: String,
    skills: Vec<String>,
    #[serde(default)]
    needs_confirmation: bool,
}

/// Overlay parcial para override por arquivo: pode substituir o fallback e/ou
/// papéis específicos (por nome), e/ou anexar papéis novos.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct OverlaySpec {
    #[serde(default)]
    fallback: Option<FallbackSpec>,
    #[serde(default)]
    roles: Vec<RoleSpec>,
}

// ───────────────────────── Estado compilado ─────────────────────────────────

/// Um pattern compilado em três formas ancoradas, uma por nível da cascata.
#[derive(Debug)]
struct CompiledPattern {
    /// `^(?:frag)$` — casamento exato (S1).
    exact: Regex,
    /// `^(?:frag)` — frag é prefixo do nome (S2).
    prefix: Regex,
    /// `\b(?:frag)` — frag começa uma palavra em qualquer ponto (S3).
    keyword: Regex,
}

/// Papel compilado: dados + patterns prontos para casar.
#[derive(Debug)]
struct CompiledRole {
    role: String,
    skills: Vec<String>,
    needs_confirmation: bool,
    patterns: Vec<CompiledPattern>,
}

impl CompiledRole {
    fn assignment(&self, matched_by: MatchStrategy) -> RoleAssignment {
        RoleAssignment {
            role: self.role.clone(),
            skills: self.skills.clone(),
            needs_confirmation: self.needs_confirmation,
            matched_by,
        }
    }
}

/// Fallback compilado.
#[derive(Debug)]
struct Fallback {
    role: String,
    skills: Vec<String>,
    needs_confirmation: bool,
}

impl Fallback {
    fn assignment(&self, matched_by: MatchStrategy) -> RoleAssignment {
        RoleAssignment {
            role: self.role.clone(),
            skills: self.skills.clone(),
            needs_confirmation: self.needs_confirmation,
            matched_by,
        }
    }
}

/// Registry de papéis pronto para inferência. Construído uma vez (compila os
/// regexes) e reutilizado; [`infer_role`](RoleRegistry::infer_role) é infalível.
#[derive(Debug)]
pub struct RoleRegistry {
    roles: Vec<CompiledRole>,
    fallback: Fallback,
    /// Captura o token dentro de `[...]` para o override explícito (S0).
    override_re: Regex,
    /// Remove qualquer `[...]` antes do casamento por nome (S1–S3).
    bracket_re: Regex,
    /// Reconhece nomes genéricos `"terminal N"`/número (S4).
    numeric_re: Regex,
}

impl RoleRegistry {
    /// Registry canônico embutido (`default-roles.yaml`).
    ///
    /// Devolve `Result` por princípio (sem `expect` em caminho de produção),
    /// ainda que o YAML embutido seja validado pelos testes deste crate.
    pub fn with_defaults() -> Result<Self, RegistryError> {
        Self::from_yaml_str(DEFAULT_REGISTRY_YAML, "<default-roles.yaml>")
    }

    /// Constrói um registry a partir de uma string YAML completa (substitui o
    /// default). `label` aparece nas mensagens de erro.
    pub fn from_yaml_str(yaml: &str, label: &str) -> Result<Self, RegistryError> {
        let spec: RegistrySpec =
            serde_yaml::from_str(yaml).map_err(|source| RegistryError::Parse {
                label: label.to_owned(),
                source: Box::new(source),
            })?;
        Self::from_spec(spec)
    }

    /// Carrega um registry completo de um arquivo `.yaml` (override por arquivo).
    pub fn from_yaml_file(path: impl AsRef<Path>) -> Result<Self, RegistryError> {
        let path = path.as_ref();
        let label = path.display().to_string();
        let src = std::fs::read_to_string(path).map_err(|source| RegistryError::Io {
            path: label.clone(),
            source,
        })?;
        Self::from_yaml_str(&src, &label)
    }

    /// Aplica um **overlay** YAML sobre este registry: substitui papéis de mesmo
    /// nome (preservando a posição na cascata), anexa papéis novos no fim e, se
    /// presente, troca o fallback. É a forma de "override por arquivo" granular.
    pub fn overlay_yaml_str(&mut self, yaml: &str, label: &str) -> Result<(), RegistryError> {
        let overlay: OverlaySpec =
            serde_yaml::from_str(yaml).map_err(|source| RegistryError::Parse {
                label: label.to_owned(),
                source: Box::new(source),
            })?;

        if let Some(fallback) = overlay.fallback {
            self.fallback = compile_fallback(fallback)?;
        }
        for role_spec in overlay.roles {
            let compiled = compile_role(role_spec)?;
            match self.roles.iter_mut().find(|r| r.role == compiled.role) {
                Some(existing) => *existing = compiled,
                None => self.roles.push(compiled),
            }
        }
        Ok(())
    }

    fn from_spec(spec: RegistrySpec) -> Result<Self, RegistryError> {
        let fallback = compile_fallback(spec.fallback)?;
        let mut roles = Vec::with_capacity(spec.roles.len());
        for role_spec in spec.roles {
            roles.push(compile_role(role_spec)?);
        }

        // Regexes estruturais (estáticos, conhecidos-bons): override, bracket e
        // numérico genérico. Compilados via `build_static_re` para herdar o
        // mesmo tratamento de erro acionável (jamais `unwrap`).
        let override_re = build_static_re(r"\[\s*([A-Za-z_][A-Za-z0-9_]*)\s*\]")?;
        let bracket_re = build_static_re(r"\[[^\]]*\]")?;
        let numeric_re = build_static_re(
            r"(?i)^(?:(?:terminal|term|tab|janela|aba|window|painel|slot)\b\s*[#:_-]?\s*)?\d+$",
        )?;

        Ok(Self {
            roles,
            fallback,
            override_re,
            bracket_re,
            numeric_re,
        })
    }

    /// Infere o papel de um terminal pelo seu nome. **Determinístico e infalível**:
    /// nomes vazios/desconhecidos caem no fallback, nunca em panic.
    pub fn infer_role(&self, terminal_name: &str) -> RoleAssignment {
        let trimmed = terminal_name.trim();

        // S0 — override explícito `[PAPEL]` (doc §2.1: sempre vence).
        if let Some(role) = self.match_override(trimmed) {
            return role.assignment(MatchStrategy::ExplicitOverride);
        }

        let had_at = trimmed.starts_with('@');
        let body = self.normalize_body(trimmed);
        if body.is_empty() {
            return self.fallback.assignment(MatchStrategy::DefaultFallback);
        }

        // S1 — exato.
        if let Some(role) = self
            .roles
            .iter()
            .find(|r| r.patterns.iter().any(|p| p.exact.is_match(&body)))
        {
            return role.assignment(MatchStrategy::ExactName);
        }

        // S2 — prefixo `@` (o pattern mais longo vence; empate → ordem do registry).
        if had_at {
            if let Some(role) = self.longest_prefix(&body) {
                return role.assignment(MatchStrategy::AtPrefix);
            }
        }

        // S3 — palavra-chave contida (início de palavra), primeira na ordem vence.
        if let Some(role) = self
            .roles
            .iter()
            .find(|r| r.patterns.iter().any(|p| p.keyword.is_match(&body)))
        {
            return role.assignment(MatchStrategy::Keyword);
        }

        // S4 — genérico numérico ("terminal N" / número) → fallback, pede confirmação.
        if self.numeric_re.is_match(&body) {
            return self.fallback.assignment(MatchStrategy::GenericNumeric);
        }

        // S5 — fallback default.
        self.fallback.assignment(MatchStrategy::DefaultFallback)
    }

    /// Nomes canônicos dos papéis, na ordem da cascata (sem o fallback).
    pub fn role_names(&self) -> impl Iterator<Item = &str> {
        self.roles.iter().map(|r| r.role.as_str())
    }

    /// Quantidade de papéis com patterns (exclui o fallback).
    pub fn len(&self) -> usize {
        self.roles.len()
    }

    /// `true` se não há nenhum papel com patterns.
    pub fn is_empty(&self) -> bool {
        self.roles.is_empty()
    }

    /// S0: primeiro `[TOKEN]` cujo `TOKEN` (maiúsculo) casa um papel canônico.
    fn match_override(&self, name: &str) -> Option<&CompiledRole> {
        for caps in self.override_re.captures_iter(name) {
            let token = caps.get(1)?.as_str().to_uppercase();
            if let Some(role) = self.roles.iter().find(|r| r.role == token) {
                return Some(role);
            }
        }
        None
    }

    /// Normaliza o nome para o casamento por pattern: remove um `@` inicial,
    /// remove qualquer `[...]`, baixa caixa e colapsa espaços (preserva espaços
    /// internos — patterns como `"dev back"` dependem deles).
    fn normalize_body(&self, trimmed: &str) -> String {
        let no_at = trimmed.strip_prefix('@').unwrap_or(trimmed);
        let no_bracket = self.bracket_re.replace_all(no_at, " ");
        no_bracket
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    }

    /// S2: papel cujo pattern é o prefixo mais longo de `body`. Empate de
    /// comprimento → papel anterior na ordem do registry (first match wins).
    fn longest_prefix(&self, body: &str) -> Option<&CompiledRole> {
        let mut best: Option<(usize, &CompiledRole)> = None;
        for role in &self.roles {
            let role_len = role
                .patterns
                .iter()
                .filter_map(|p| p.prefix.find(body).map(|m| m.end()))
                .max()
                .unwrap_or(0);
            if role_len == 0 {
                continue;
            }
            let beats = match best {
                Some((best_len, _)) => role_len > best_len,
                None => true,
            };
            if beats {
                best = Some((role_len, role));
            }
        }
        best.map(|(_, role)| role)
    }
}

// ───────────────────────── Compilação de patterns ───────────────────────────

/// Compila as três formas ancoradas de um fragmento regex de pattern.
fn compile_pattern(role: &str, frag: &str) -> Result<CompiledPattern, RegistryError> {
    let make = |anchored: String| {
        Regex::new(&anchored).map_err(|source| RegistryError::Pattern {
            role: role.to_owned(),
            pattern: frag.to_owned(),
            source: Box::new(source),
        })
    };
    Ok(CompiledPattern {
        exact: make(format!("(?i)^(?:{frag})$"))?,
        prefix: make(format!("(?i)^(?:{frag})"))?,
        keyword: make(format!(r"(?i)\b(?:{frag})"))?,
    })
}

fn compile_role(spec: RoleSpec) -> Result<CompiledRole, RegistryError> {
    if spec.role.trim().is_empty() {
        return Err(RegistryError::EmptyRole);
    }
    let mut patterns = Vec::with_capacity(spec.patterns.len());
    for frag in &spec.patterns {
        patterns.push(compile_pattern(&spec.role, frag)?);
    }
    Ok(CompiledRole {
        role: spec.role,
        skills: spec.skills,
        needs_confirmation: spec.needs_confirmation,
        patterns,
    })
}

fn compile_fallback(spec: FallbackSpec) -> Result<Fallback, RegistryError> {
    if spec.role.trim().is_empty() {
        return Err(RegistryError::EmptyRole);
    }
    Ok(Fallback {
        role: spec.role,
        skills: spec.skills,
        needs_confirmation: spec.needs_confirmation,
    })
}

/// Compila um regex estático interno; erro acionável em vez de panic.
fn build_static_re(pattern: &str) -> Result<Regex, RegistryError> {
    Regex::new(pattern).map_err(|source| RegistryError::Pattern {
        role: "<interno>".to_owned(),
        pattern: pattern.to_owned(),
        source: Box::new(source),
    })
}

// ───────────────────────── Erros ────────────────────────────────────────────

/// Erros de construção do registry. Sempre acionáveis (carregam contexto),
/// nunca um panic. [`RoleRegistry::infer_role`] é separadamente infalível.
#[derive(Debug, Error)]
pub enum RegistryError {
    /// Falha de I/O ao ler o arquivo de registry.
    #[error("falha de I/O ao acessar '{path}': {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// YAML malformado ou faltando campo obrigatório.
    #[error("YAML de registry inválido em '{label}': {source}")]
    Parse {
        label: String,
        #[source]
        source: Box<serde_yaml::Error>,
    },

    /// Um pattern regex não compilou (registry inválido).
    #[error("pattern inválido '{pattern}' no papel '{role}': {source}")]
    Pattern {
        role: String,
        pattern: String,
        #[source]
        source: Box<regex::Error>,
    },

    /// Um papel (ou o fallback) veio com `role` vazio.
    #[error("campo 'role' vazio no registry")]
    EmptyRole,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> RoleRegistry {
        RoleRegistry::with_defaults().expect("default-roles.yaml embutido deve compilar")
    }

    fn infer(name: &str) -> RoleAssignment {
        registry().infer_role(name)
    }

    // ── O registry default carrega e tem a forma canônica do doc §2.2 ────────
    #[test]
    fn default_registry_builds_with_canonical_roles() {
        let reg = registry();
        let names: Vec<&str> = reg.role_names().collect();
        assert_eq!(
            names,
            vec![
                "MAESTRO",
                "ARQUITETO",
                "FRONTEND",
                "BACKEND",
                "LLM_ENGINEER",
                "DATA_ENGINEER",
                "UIUX_DESIGNER",
                "WRITER",
                "QA",
                "BUG_FIXER",
                "CURADOR",
                "AUTOMATOR",
            ],
            "ordem da cascata = ordem da tabela canônica (first match wins)"
        );
        assert_eq!(reg.len(), 12);
        assert!(!reg.is_empty());
    }

    // ── S0: override explícito `[PAPEL]` sempre vence ────────────────────────
    #[test]
    fn strategy_explicit_override_always_wins() {
        // O corpo tem "backend", mas o override força QA (doc §2.1).
        let a = infer("Meu Terminal de Backend [QA]");
        assert_eq!(a.role, "QA");
        assert_eq!(a.matched_by, MatchStrategy::ExplicitOverride);
        assert_eq!(a.skills, vec!["e2e-product-qa"]);
        assert!(!a.needs_confirmation);

        // Override para ARQUITETO mesmo num nome de pesquisa.
        assert_eq!(infer("Pesquisa de mercado [ARQUITETO]").role, "ARQUITETO");

        // Token desconhecido em colchetes NÃO é override: cai na cascata normal.
        let unknown = infer("@Backend [NAO_EXISTE]");
        assert_eq!(unknown.role, "BACKEND");
        assert_ne!(unknown.matched_by, MatchStrategy::ExplicitOverride);
    }

    // ── S1: nome exato — "@Arquiteto" E "Arquiteto" → ARQUITETO, NUNCA MAESTRO ─
    #[test]
    fn strategy_exact_name_arquiteto_never_maestro() {
        for name in ["@Arquiteto", "Arquiteto", "  @ARQUITETO  ", "architect"] {
            let a = infer(name);
            assert_eq!(a.role, "ARQUITETO", "{name:?} deve ser ARQUITETO");
            assert_ne!(a.role, "MAESTRO", "{name:?} JAMAIS pode ser MAESTRO");
            assert_eq!(a.matched_by, MatchStrategy::ExactName);
            assert_eq!(a.skills, vec!["senior-architect"]);
            assert!(!a.needs_confirmation);
        }
    }

    // ── S2: prefixo `@` com especialidade — a parte após `@` define o papel ──
    #[test]
    fn strategy_at_prefix_with_specialty() {
        // "@Dev Backend": casa o pattern-prefixo "dev back" → BACKEND.
        let backend = infer("@Dev Backend");
        assert_eq!(backend.role, "BACKEND");
        assert_eq!(backend.matched_by, MatchStrategy::AtPrefix);
        assert_eq!(backend.skills, vec!["senior-backend", "tomik-db-doctrine"]);

        // "@Arquiteto Backend Helper": o `@`-prefixo é ARQUITETO; "backend" no
        // meio NÃO pode vencer (é exatamente o que a cascata previne).
        let arq = infer("@Arquiteto Backend Helper");
        assert_eq!(arq.role, "ARQUITETO");
        assert_eq!(arq.matched_by, MatchStrategy::AtPrefix);

        // "@Writer LP": papel WRITER, especialidade ignorada.
        assert_eq!(infer("@Writer LP").role, "WRITER");
    }

    // ── S3: palavra-chave contida (sem `@`, início de palavra) ───────────────
    #[test]
    fn strategy_keyword_contained() {
        let backend = infer("preciso construir o backend da api");
        assert_eq!(backend.role, "BACKEND");
        assert_eq!(backend.matched_by, MatchStrategy::Keyword);

        assert_eq!(infer("fazer a interface em react").role, "FRONTEND");
        // sufixo/plural ainda casa (início de palavra): "designer" → "design".
        let ux = infer("nosso designer chefe");
        assert_eq!(ux.role, "UIUX_DESIGNER");
        assert_eq!(ux.matched_by, MatchStrategy::Keyword);
    }

    // ── S4: genérico numérico → DEVELOPER + needs_confirmation=true ──────────
    #[test]
    fn strategy_generic_numeric_needs_confirmation() {
        for name in [
            "terminal 3",
            "Terminal 3",
            "3",
            "terminal-2",
            "term 5",
            "@7",
        ] {
            let a = infer(name);
            assert_eq!(a.role, "DEVELOPER", "{name:?} deve cair em DEVELOPER");
            assert!(a.needs_confirmation, "{name:?} deve pedir confirmação");
            assert_eq!(a.matched_by, MatchStrategy::GenericNumeric, "{name:?}");
            assert_eq!(a.skills, vec!["senior-frontend", "senior-backend"]);
        }
    }

    // ── S5: fallback default (nome desconhecido, não-numérico) ───────────────
    #[test]
    fn strategy_default_fallback() {
        for name in ["Banana", "xpto qualquer", "", "   ", "@"] {
            let a = infer(name);
            assert_eq!(a.role, "DEVELOPER", "{name:?} deve cair em DEVELOPER");
            assert!(a.needs_confirmation, "{name:?} deve pedir confirmação");
            assert_eq!(a.matched_by, MatchStrategy::DefaultFallback, "{name:?}");
        }
    }

    // ── Matriz: nomes conhecidos → papel + skills certos ─────────────────────
    #[test]
    fn known_names_map_to_role_and_skills() {
        let cases: &[(&str, &str, &[&str])] = &[
            // BUG-2 dogfood r1: a skill do MAESTRO é a interna `lina-orchestration` —
            // `maestri-orchestrator` é estrangeira e o guard do Lina a bloqueia.
            ("@Maestro", "MAESTRO", &["lina-orchestration"]),
            (
                "@Frontend",
                "FRONTEND",
                &["senior-frontend", "frontend-design", "unique-ui-design"],
            ),
            (
                "@LLM",
                "LLM_ENGINEER",
                &["senior-prompt-engineer", "ai-agent-qa"],
            ),
            (
                "@DB",
                "DATA_ENGINEER",
                &["tomik-db-doctrine", "agent-runtime-data-split"],
            ),
            (
                "@Design",
                "UIUX_DESIGNER",
                &["ui-ux-pro-max", "ui-ux-auditor", "unique-ui-design"],
            ),
            ("@QA", "QA", &["e2e-product-qa"]),
            ("@Bug", "BUG_FIXER", &["autonomous-bug-fixer"]),
            ("@Curador", "CURADOR", &["tino-refresh", "find-skills"]),
            ("@Webhook", "AUTOMATOR", &["mcp-builder"]),
        ];
        for (name, role, skills) in cases {
            let a = infer(name);
            assert_eq!(&a.role, role, "{name:?}");
            assert_eq!(a.skills, *skills, "{name:?}");
            assert!(!a.needs_confirmation, "{name:?} é papel declarado");
        }
    }

    // ── Desambiguação UI vs UI/UX pela cascata (exato vence contido) ─────────
    #[test]
    fn ui_versus_uiux_disambiguation() {
        // "ui ux" casa o regex exato `ui.?ux` (UIUX) antes de "ui" (FRONTEND).
        assert_eq!(infer("ui ux").role, "UIUX_DESIGNER");
        assert_eq!(infer("@UX").role, "UIUX_DESIGNER");
        // "ui" e "react" sozinhos são FRONTEND (doc §2.2).
        assert_eq!(infer("@UI").role, "FRONTEND");
        assert_eq!(infer("react").role, "FRONTEND");
    }

    // ── Sem falso-positivo de substring (fronteira de palavra) ───────────────
    #[test]
    fn no_substring_false_positives() {
        // "research" contém "arch", mas não como palavra → não é ARQUITETO.
        assert_ne!(infer("research lab").role, "ARQUITETO");
        // "build" contém "ui", mas não como palavra → não é FRONTEND.
        assert_ne!(infer("build pipeline").role, "FRONTEND");
    }

    // ── Determinismo: mesma entrada, mesma saída ─────────────────────────────
    #[test]
    fn inference_is_deterministic() {
        let reg = registry();
        for name in ["@Arquiteto", "terminal 3", "ui ux", "Banana"] {
            assert_eq!(reg.infer_role(name), reg.infer_role(name), "{name:?}");
        }
    }

    // ── Override por arquivo (overlay): substitui papel e anexa novo ─────────
    #[test]
    fn overlay_overrides_role_and_appends_new() {
        let mut reg = registry();
        reg.overlay_yaml_str(
            r#"
            roles:
              - role: ARQUITETO
                patterns: [arquiteto, architect, arch]
                skills: [senior-architect, senior-prompt-engineer]
              - role: REVISOR
                patterns: [revisor, reviewer, review]
                skills: [code-review]
                needs_confirmation: true
            "#,
            "<overlay-teste>",
        )
        .expect("overlay válido deve aplicar");

        // ARQUITETO mantém a posição, mas com skills sobrescritas.
        let arq = reg.infer_role("@Arquiteto");
        assert_eq!(arq.role, "ARQUITETO");
        assert_eq!(
            arq.skills,
            vec!["senior-architect", "senior-prompt-engineer"]
        );

        // Papel novo passa a ser descoberto.
        let rev = reg.infer_role("@Revisor");
        assert_eq!(rev.role, "REVISOR");
        assert_eq!(rev.skills, vec!["code-review"]);
        assert!(rev.needs_confirmation);
        assert_eq!(reg.len(), 13);
    }

    // ── Erros acionáveis, sem panic ──────────────────────────────────────────
    #[test]
    fn invalid_yaml_is_a_clear_error() {
        let err = RoleRegistry::from_yaml_str("roles: [", "<inline>")
            .expect_err("YAML quebrado deve falhar");
        assert!(matches!(err, RegistryError::Parse { .. }));
        assert!(err.to_string().contains("<inline>"));
    }

    #[test]
    fn invalid_regex_pattern_is_a_clear_error() {
        let yaml = r#"
            fallback:
              role: DEVELOPER
              skills: []
            roles:
              - role: BROKEN
                patterns: ["("]
                skills: []
        "#;
        let err = RoleRegistry::from_yaml_str(yaml, "<inline>")
            .expect_err("pattern regex inválido deve falhar");
        assert!(matches!(err, RegistryError::Pattern { .. }));
        assert!(err.to_string().contains("BROKEN"));
    }

    #[test]
    fn missing_yaml_file_is_an_io_error() {
        let err = RoleRegistry::from_yaml_file("/caminho/que/nao/existe-12345.yaml")
            .expect_err("arquivo inexistente deve falhar");
        assert!(matches!(err, RegistryError::Io { .. }));
    }

    #[test]
    fn empty_role_name_is_rejected() {
        let yaml = r#"
            fallback:
              role: DEVELOPER
              skills: []
            roles:
              - role: "  "
                patterns: [x]
                skills: []
        "#;
        let err =
            RoleRegistry::from_yaml_str(yaml, "<inline>").expect_err("role vazio deve falhar");
        assert!(matches!(err, RegistryError::EmptyRole));
    }
}
