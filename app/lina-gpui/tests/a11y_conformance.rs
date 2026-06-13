//! **F2-2 / r6 — guard de conformidade ADR 0028:** um componente que **comunica ESTADO** (status
//! que muda e deve ser falado SEM foco) tem de NASCER compondo o live-region
//! (`a11y_live::live_region`). Quebra sozinho na CI se alguém comunicar estado fora do live-region.
//! Espírito da catraca de tokens (`token_ratchet.rs`): matchers PINADOS, allowlist que só DESCE,
//! prova por MUTAÇÃO.
//!
//! ## Duas réguas, dois escopos
//! 1. **DURA (Role de estado ambiente) — `src/**.rs` INTEIRO.** Um arquivo que usa, EM CÓDIGO,
//!    `Role::Status`/`Role::Alert`/`A11yRole::Status`/`A11yRole::Alert`/`Politeness` (a semântica de
//!    auto-anúncio sem foco) **tem** de compor o live-region (`live_region(`/`live_region_element(`/
//!    `live_announce(`). Esses papéis SÃO a live-region; usá-los sem o nó é o retrofit que o ADR 0028
//!    proíbe. Escopo `src/**` (não só `src/ui/`): o ESTADO é renderizado nos call-sites
//!    (`attention_ui`/`main`/`canvas`) — fechar só o catálogo deixava a fronteira aberta para o
//!    call-site FUTURO (achado MÉDIO do Arquiteto-revisor, PASS 91; é o próprio achado-1 que esta
//!    régua nomeou). Exceção ÚNICA: [`LIVE_REGION_DEFINER`] — o módulo que DEFINE o Element.
//! 2. **CATRACA (cor-de-estado) — `src/ui/` (catálogo).** Um componente do catálogo que usa
//!    `state.success`/`warning`/`danger` ou compõe live_region, ou está em [`FOCUS_STATE_EXEMPTIONS`]
//!    com motivo (cor anunciada NO FOCO: botão destrutivo, erro de campo). A lista só desce; exceção
//!    stale (graduou a anunciador / sumiu / parou de usar cor) é reportada. Fora do catálogo a
//!    cor-de-estado tem usos legítimos demais (bordas, erros, CTAs) — a régua dura é a que vale lá.
//!
//! ## Calibração anti-falso-positivo (o ponto da story)
//! - **Comentários** que mapeiam o mecanismo (`// o Role::Status cru não auto-anuncia`) NÃO contam:
//!   o scan da régua dura roda sobre o código com os comentários de linha removidos.
//! - **Definidor** (`a11y_live.rs`) referencia os markers para CONSTRUIR o Element → isento.
//! - **Família de composição**: `live_region_element(`/`live_announce(` contam como composição (são a
//!   API pública que produz o nó) — senão um call-site que compõe via helper delegador cairia à toa.
//! - **Fronteira de palavra**: `Role::Alert` ≠ `Role::AlertDialog`; `A11yRole::Status` é casado pelo
//!   próprio marker (a forma aliasada do codebase), não como sufixo de `Role::Status`.

use std::path::Path;

// ───────────────────────── Matchers PINADOS (não enfraquecer em silêncio) ─────────────────────────

/// Tokens de COR-DE-ESTADO (régua catraca, só no catálogo `src/ui/`).
const STATE_COLOR_TOKENS: &[&str] = &["state.success", "state.warning", "state.danger"];

/// Semântica de LIVE-REGION AMBIENTE (régua dura, `src/**`). Cobre a forma BARE (`Role::Status`, como
/// `div().role(Role::Status)`) e a ALIASADA do codebase (`A11yRole::Status`, do custom Element), além
/// de `Politeness` (a cortesia que só faz sentido compondo o live-region).
const AMBIENT_ANNOUNCE_MARKERS: &[&str] = &[
    "Role::Status",
    "Role::Alert",
    "A11yRole::Status",
    "A11yRole::Alert",
    "Politeness",
];

/// Família de composição do live-region (ADR 0028 caminho a): a CHAMADA que produz o nó
/// auto-anunciável — direta (`live_region(`) ou via os helpers públicos (`live_region_element(`,
/// `live_announce(`). Terminam em `(` — discriminam a chamada de uma menção em comentário.
const COMPOSE_LIVE_REGION: &[&str] = &["live_region(", "live_region_element(", "live_announce("];

/// O DEFINIDOR do live-region: o módulo que DECLARA o custom `Element` + `Politeness` + o mapa de
/// `Role`. Referencia os markers estruturalmente (para CONSTRUIR o nó), não como consumidor — isento
/// da régua dura. Exigir que o definidor "componha" seria circular.
const LIVE_REGION_DEFINER: &[&str] = &["a11y_live.rs"];

/// Arquivos exceção da CATRACA (catálogo): usam cor-de-estado mas anunciam NO FOCO (botão/campo), não
/// como status ambiente. Só desce; exceção stale é reportada.
const FOCUS_STATE_EXEMPTIONS: &[(&str, &str)] = &[
    (
        "button.rs",
        "state.danger = cor da AÇÃO destrutiva (Role::Button), anunciada NO FOCO como botão — não status ambiente",
    ),
    (
        "input.rs",
        "state.danger = erro de validação do campo, anunciado NO FOCO via a11y do próprio campo — não live-region ambiente",
    ),
];

// ───────────────────────────── Detecção (fronteira de palavra) ─────────────────────────────

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// `true` se `token` ocorre em `src` como PALAVRA (vizinhos não-identificadores), de modo que
/// `Role::Alert` case `Role::Alert,`/`Role::Alert)` mas NÃO `Role::AlertDialog`, e `state.danger` não
/// case `state.dangerous`. Tokens são ASCII → o fatiamento por byte é seguro.
fn references(src: &str, token: &str) -> bool {
    src.match_indices(token).any(|(i, _)| {
        let before_ok = src[..i].chars().next_back().is_none_or(|c| !is_ident(c));
        let after_ok = src[i + token.len()..]
            .chars()
            .next()
            .is_none_or(|c| !is_ident(c));
        before_ok && after_ok
    })
}

/// O código com os comentários de LINHA (`//`, `///`, `//!`) removidos, para a régua dura não cair em
/// menções de mecanismo dentro de comentários. Heurística: corta cada linha no 1º `//` (suficiente —
/// nenhum marker do codebase vive antes de um `//` colado num literal; `::` ≠ `//`). Comentários de
/// BLOCO (`/* */`) não são removidos: o baseline não tem marker neles (provado pelo teste de baseline).
fn strip_line_comments(src: &str) -> String {
    src.lines()
        .map(|line| &line[..line.find("//").unwrap_or(line.len())])
        .collect::<Vec<_>>()
        .join("\n")
}

fn uses_state_color(src: &str) -> bool {
    STATE_COLOR_TOKENS.iter().any(|t| references(src, t))
}

fn declares_ambient_announce(src: &str) -> bool {
    AMBIENT_ANNOUNCE_MARKERS.iter().any(|t| references(src, t))
}

fn composes_live_region(src: &str) -> bool {
    COMPOSE_LIVE_REGION.iter().any(|c| src.contains(c))
}

fn matched_state_tokens(src: &str) -> String {
    STATE_COLOR_TOKENS
        .iter()
        .filter(|t| references(src, t))
        .copied()
        .collect::<Vec<_>>()
        .join("/")
}

fn is_exempt(name: &str) -> bool {
    FOCUS_STATE_EXEMPTIONS.iter().any(|(n, _)| *n == name)
}

fn is_definer(name: &str) -> bool {
    LIVE_REGION_DEFINER.contains(&name)
}

// ─────────────────────────────────── As réguas (puras) ───────────────────────────────────

/// **Régua DURA** (`src/**`): usar a semântica de anúncio ambiente EM CÓDIGO obriga compor o
/// live-region. Comentários removidos; definidor isento. Devolve a violação de cada arquivo.
fn ambient_hard_violations(files: &[(String, String)]) -> Vec<String> {
    let mut violations = Vec::new();
    for (name, src) in files {
        if is_definer(name) {
            continue; // o módulo que DEFINE o Element referencia os markers estruturalmente
        }
        let code = strip_line_comments(src);
        if declares_ambient_announce(&code) && !composes_live_region(&code) {
            violations.push(format!(
                "{name}: usa a semântica de anúncio AMBIENTE (Role/A11yRole::Status|Alert ou \
                 Politeness) em CÓDIGO mas NÃO compõe o live-region (live_region(...) / \
                 live_region_element(...) / live_announce(...)). ADR 0028: o estado deve NASCER sobre o \
                 live-region — retrofit proibido. (Se este arquivo DEFINE o Element, adicione-o a \
                 LIVE_REGION_DEFINER.)"
            ));
        }
    }
    violations
}

/// **Régua CATRACA** (`src/ui/`): usar cor-de-estado obriga compor o live-region OU estar na
/// allowlist justificada (cor anunciada no FOCO).
fn state_color_ratchet_violations(files: &[(String, String)]) -> Vec<String> {
    let mut violations = Vec::new();
    for (name, src) in files {
        let code = strip_line_comments(src);
        if uses_state_color(&code) && !composes_live_region(&code) && !is_exempt(name) {
            violations.push(format!(
                "{name}: comunica estado pela cor ({}) no catálogo mas NÃO compõe live_region(...) e \
                 não está na allowlist. Se é STATUS ambiente, NASÇA sobre a11y_live::live_region (ADR \
                 0028); se a cor é de AÇÃO/erro anunciado NO FOCO, adicione `(\"{name}\", \"<motivo>\")` \
                 a FOCUS_STATE_EXEMPTIONS.",
                matched_state_tokens(&code)
            ));
        }
    }
    violations
}

/// Anti-stale (a allowlist da catraca só desce): toda exceção tem de continuar SE APLICANDO — arquivo
/// que sumiu, deixou de usar cor-de-estado, ou graduou a anunciador (compõe) → exceção morta.
fn stale_exemption_violations(files: &[(String, String)]) -> Vec<String> {
    let mut violations = Vec::new();
    for (name, reason) in FOCUS_STATE_EXEMPTIONS {
        match files.iter().find(|(n, _)| n == name) {
            None => violations.push(format!(
                "{name}: exceção aponta para arquivo inexistente em src/ui/ — remova de \
                 FOCUS_STATE_EXEMPTIONS (motivo registrado: {reason})."
            )),
            Some((_, src)) => {
                let code = strip_line_comments(src);
                if composes_live_region(&code) {
                    violations.push(format!(
                        "{name}: exceção STALE — o arquivo agora COMPÕE live_region (graduou a \
                         anunciador ambiente). Remova de FOCUS_STATE_EXEMPTIONS."
                    ));
                } else if !uses_state_color(&code) {
                    violations.push(format!(
                        "{name}: exceção STALE — o arquivo não usa mais cor-de-estado, não precisa de \
                         exceção. Remova de FOCUS_STATE_EXEMPTIONS."
                    ));
                }
            }
        }
    }
    violations
}

// ─────────────────────────────────── Leitura do fonte ───────────────────────────────────

fn manifest() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// `(nome, fonte)` de cada `*.rs` sob um diretório (recursivo), exceto `mod.rs`.
fn rust_sources(root: &Path) -> Vec<(String, String)> {
    let mut pending = vec![root.to_path_buf()];
    let mut out = Vec::new();
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let path = entry.expect("entry de fonte").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if name != "mod.rs" {
                    let src = std::fs::read_to_string(&path).expect("ler fonte");
                    out.push((name, src));
                }
            }
        }
    }
    out
}

/// Só o catálogo de componentes (`src/ui/`) — escopo da régua catraca.
fn ui_sources() -> Vec<(String, String)> {
    rust_sources(&manifest().join("src/ui"))
}

/// Toda a fonte do shell (`src/**`) — escopo da régua dura (call-sites incluídos).
fn all_src_sources() -> Vec<(String, String)> {
    rust_sources(&manifest().join("src"))
}

// ─────────────────────────────────────── Os testes-guard ───────────────────────────────────────

/// **Catálogo (`src/ui/`):** régua dura + catraca + anti-stale, todas verdes.
#[test]
fn ui_catalog_conforms_to_adr_0028() {
    let ui = ui_sources();
    let mut v = ambient_hard_violations(&ui);
    v.extend(state_color_ratchet_violations(&ui));
    v.extend(stale_exemption_violations(&ui));
    assert!(
        v.is_empty(),
        "Conformidade ADR 0028 no catálogo violada — {} ocorrência(s):\n  - {}",
        v.len(),
        v.join("\n  - ")
    );
}

/// **Call-sites (`src/**`):** a régua DURA vale ONDE o estado é renderizado, não só no catálogo —
/// a fronteira do achado MÉDIO (Arquiteto, PASS 91) fechada. Quebra se um call-site futuro usar
/// Role de estado sem compor o live-region.
#[test]
fn call_sites_conform_to_adr_0028_hard_rule() {
    let v = ambient_hard_violations(&all_src_sources());
    assert!(
        v.is_empty(),
        "Régua dura ADR 0028 violada FORA do catálogo — {} ocorrência(s):\n  - {}",
        v.len(),
        v.join("\n  - ")
    );
}

// ─────────────────────────── Calibração por MUTAÇÃO (false-neg / false-pos) ───────────────────────────

/// **False-NEGATIVO (catraca):** componente NOVO de catálogo que comunica estado mas esquece o
/// live-region cai.
#[test]
fn mutation_new_state_component_without_live_region_is_caught() {
    let mutant = (
        "badge_novo.rs".to_string(),
        "fn render() { div().bg(rgb(t.state.success)).child(text!(label)) }".to_string(),
    );
    let v = state_color_ratchet_violations(&[mutant]);
    assert!(
        v.iter()
            .any(|m| m.contains("badge_novo.rs") && m.contains("não está na allowlist")),
        "componente novo de estado sem live_region DEVE cair: {v:?}"
    );
}

/// **False-POSITIVO (catraca):** o mesmo componente compondo o live-region NÃO cai.
#[test]
fn mutation_compliant_state_component_passes() {
    let ok = (
        "badge_novo.rs".to_string(),
        "let pill = div().bg(rgb(t.state.success)); live_region(id, msg, Politeness::Polite, pill)"
            .to_string(),
    );
    assert!(
        state_color_ratchet_violations(std::slice::from_ref(&ok)).is_empty()
            && ambient_hard_violations(&[ok]).is_empty(),
        "componente de estado que COMPÕE live_region não pode cair"
    );
}

/// **Régua DURA — a mutação central da story:** um `div().role(Role::Status)` CRU num call-site
/// (fora do catálogo) sem compor o live-region DEVE cair.
#[test]
fn mutation_raw_state_role_in_a_call_site_is_caught() {
    let mutant = (
        "attention_ui.rs".to_string(),
        "div().role(Role::Status).child(text!(\"3 pendencias\"))".to_string(),
    );
    let v = ambient_hard_violations(&[mutant]);
    assert!(
        v.iter()
            .any(|m| m.contains("attention_ui.rs") && m.contains("AMBIENTE")),
        "Role::Status cru num call-site sem composição DEVE cair: {v:?}"
    );
}

/// **Régua DURA — forma aliasada:** `A11yRole::Alert` cru (custom Element fora do definidor) também
/// cai. (No definidor seria isento; aqui o arquivo não é o definidor.)
#[test]
fn mutation_aliased_role_outside_definer_is_caught() {
    let mutant = (
        "novo_elemento.rs".to_string(),
        "fn a11y_role() -> A11yRole { A11yRole::Alert }".to_string(),
    );
    assert!(
        !ambient_hard_violations(&[mutant]).is_empty(),
        "A11yRole::Alert cru fora do definidor DEVE cair"
    );
}

/// **Calibração de COMENTÁRIO:** uma MENÇÃO em comentário (mecanismo) NÃO dispara a régua dura.
#[test]
fn comment_mention_of_state_role_does_not_trip_the_hard_rule() {
    let only_comment = (
        "main.rs".to_string(),
        "// W4-6: a live-region (Role::Status) entra na cena.\nlet x = 1;".to_string(),
    );
    assert!(
        ambient_hard_violations(&[only_comment]).is_empty(),
        "menção a Role::Status em comentário não pode cair (strip de //)"
    );
}

/// **Calibração de DEFINIDOR:** `a11y_live.rs` usa os markers para DEFINIR o Element — isento.
#[test]
fn definer_module_is_exempt_from_the_hard_rule() {
    let definer = (
        "a11y_live.rs".to_string(),
        "fn role() -> A11yRole { match self { Politeness::Polite => A11yRole::Status } }"
            .to_string(),
    );
    assert!(
        ambient_hard_violations(&[definer]).is_empty(),
        "o definidor (a11y_live.rs) é isento da régua dura"
    );
}

/// **Calibração de COMPOSIÇÃO via helper:** compor pelo `live_region_element(` (delegador) conta
/// como composição — um call-site assim NÃO cai mesmo declarando a cortesia.
#[test]
fn composition_via_helper_counts_as_composing() {
    let via_helper = (
        "main.rs".to_string(),
        "let p = Politeness::Polite; a11y::live_region_element(msg)".to_string(),
    );
    assert!(
        ambient_hard_violations(&[via_helper]).is_empty(),
        "composição via live_region_element(...) satisfaz a régua dura"
    );
}

/// **Calibração de substring:** `Role::AlertDialog` (modal, foco) ≠ `Role::Alert`.
#[test]
fn alertdialog_role_is_not_an_ambient_announce_marker() {
    assert!(
        !declares_ambient_announce("panel.role(Role::AlertDialog)"),
        "Role::AlertDialog não é Role::Alert (fronteira de palavra)"
    );
    assert!(declares_ambient_announce("Role::Alert,"));
    assert!(!references("state.dangerous_thing", "state.danger"));
}

/// **False-POSITIVO:** container estrutural (sem cor-de-estado, sem papel de anúncio) não cai.
#[test]
fn structural_component_without_state_is_not_flagged() {
    let panel = (
        "panel.rs".to_string(),
        "div().bg(rgb(t.surface.raised)).p(px(pad))".to_string(),
    );
    assert!(
        ambient_hard_violations(std::slice::from_ref(&panel)).is_empty()
            && state_color_ratchet_violations(&[panel]).is_empty(),
        "container sem estado nem papel de anúncio não cai"
    );
}

/// **Anti-stale:** exceção cujo arquivo graduou a anunciador (passou a compor) é reportada.
#[test]
fn stale_exemption_that_now_composes_live_region_is_caught() {
    let graduated = (
        "input.rs".to_string(),
        "let b = t.state.danger; live_region(id, msg, Politeness::Polite, pill)".to_string(),
    );
    let v = stale_exemption_violations(&[graduated]);
    assert!(
        v.iter()
            .any(|m| m.contains("input.rs") && m.contains("STALE")),
        "exceção que graduou a anunciador deve ser marcada STALE: {v:?}"
    );
}

/// **Anti-stale:** exceção cujo arquivo sumiu de `src/ui/` é reportada.
#[test]
fn exemption_for_missing_file_is_caught() {
    let v = stale_exemption_violations(&[]);
    assert!(
        v.iter()
            .any(|m| m.contains("button.rs") && m.contains("inexistente"))
            && v.iter()
                .any(|m| m.contains("input.rs") && m.contains("inexistente")),
        "exceção apontando para arquivo ausente deve cair: {v:?}"
    );
}

/// **Matchers PINADOS** (anti-enfraquecimento silencioso): mudar exige tocar este teste.
#[test]
fn matchers_are_pinned() {
    assert_eq!(
        STATE_COLOR_TOKENS,
        &["state.success", "state.warning", "state.danger"]
    );
    assert_eq!(
        AMBIENT_ANNOUNCE_MARKERS,
        &[
            "Role::Status",
            "Role::Alert",
            "A11yRole::Status",
            "A11yRole::Alert",
            "Politeness"
        ]
    );
    assert_eq!(
        COMPOSE_LIVE_REGION,
        &["live_region(", "live_region_element(", "live_announce("]
    );
    assert_eq!(LIVE_REGION_DEFINER, &["a11y_live.rs"]);
    assert_eq!(
        FOCUS_STATE_EXEMPTIONS
            .iter()
            .map(|(n, _)| *n)
            .collect::<Vec<_>>(),
        vec!["button.rs", "input.rs"]
    );
}
