//! **F2-2 / r6 — guard de conformidade ADR 0028:** um componente de `src/ui/` que **comunica
//! ESTADO** (status que muda e deve ser falado SEM foco) tem de NASCER compondo o live-region
//! (`a11y_live::live_region`). A invariante existia só na cabeça do revisor (achado 1 do
//! Arquiteto-revisor, PASS 90: "vale onde foi escrita, fronteira aberta para componente futuro").
//! Este teste a torna AUTOMÁTICA — quebra sozinho na CI se um componente novo comunicar estado fora
//! do live-region. Espírito da catraca de tokens (`token_ratchet.rs`): matchers PINADOS, allowlist
//! que só DESCE, e prova por MUTAÇÃO.
//!
//! ## Duas réguas (a calibração é o ponto)
//! 1. **DURA (sem exceção)** — um arquivo que declara a SEMÂNTICA de anúncio ambiente
//!    (`Role::Status` / `Role::Alert` / `Politeness`) **tem** de compor `live_region(...)`. Esses
//!    papéis SÃO a live-region; usá-los sem o nó é exatamente o retrofit que o ADR 0028 proíbe.
//! 2. **CATRACA (com allowlist justificada)** — um arquivo que usa **cor-de-estado**
//!    (`state.success`/`warning`/`danger`) ou compõe `live_region`, ou está em
//!    [`FOCUS_STATE_EXEMPTIONS`] com um motivo. A exceção é para cor de estado anunciada **no FOCO**
//!    (botão destrutivo, erro de campo) — NÃO status ambiente. A lista só desce: adicionar exige
//!    revisão; uma exceção que graduou a anunciador (passou a compor) ou deixou de usar cor-de-estado
//!    é STALE e o teste manda removê-la (anti-bless, como os resets `px(0)` da catraca).
//!
//! ## Por que ler o FONTE (e não a API)
//! O guard varre `src/ui/**.rs` como TEXTO (igual ao lint de cor e à catraca) — assim ele pega o
//! componente que **esqueceu** o live-region, que por definição não tem como ser detectado pela API
//! (o bug é a ausência). Fronteira de palavra nos matchers (`Role::Alert` ≠ `Role::AlertDialog`)
//! evita o falso-positivo de substring.

use std::path::{Path, PathBuf};

// ───────────────────────── Matchers PINADOS (não enfraquecer em silêncio) ─────────────────────────

/// Tokens de COR-DE-ESTADO: usar um destes = o componente comunica estado pela cor (WCAG à parte).
const STATE_COLOR_TOKENS: &[&str] = &["state.success", "state.warning", "state.danger"];

/// Semântica de LIVE-REGION AMBIENTE (auto-anúncio SEM foco): declarar um destes obriga compor o nó,
/// sem exceção. `Role::Status` (polite) / `Role::Alert` (assertive) / `Politeness` vêm do
/// `a11y_live` — são a própria declaração de "isto é uma live-region".
const AMBIENT_ANNOUNCE_MARKERS: &[&str] = &["Role::Status", "Role::Alert", "Politeness"];

/// A composição do live-region (ADR 0028 caminho a): a CHAMADA que envolve o visual no `Element`.
/// Termina em `(` — discrimina a chamada de uma mera menção em comentário.
const COMPOSE_LIVE_REGION: &str = "live_region(";

/// Arquivos exceção (baseline pinado): usam cor-de-estado mas NÃO são anunciadores AMBIENTE — o
/// estado ali é comunicado NO FOCO (botão/campo), não como status que muda sozinho. A lista só desce.
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

/// `true` se `token` ocorre em `src` como PALAVRA (vizinhos não-identificadores dos dois lados), de
/// modo que `Role::Alert` case `Role::Alert,`/`Role::Alert)` mas NÃO `Role::AlertDialog`, e
/// `state.danger` não case `state.dangerous`. Tokens são ASCII → o fatiamento por byte é seguro.
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

fn uses_state_color(src: &str) -> bool {
    STATE_COLOR_TOKENS.iter().any(|t| references(src, t))
}

fn declares_ambient_announce(src: &str) -> bool {
    AMBIENT_ANNOUNCE_MARKERS.iter().any(|t| references(src, t))
}

fn composes_live_region(src: &str) -> bool {
    src.contains(COMPOSE_LIVE_REGION)
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

// ─────────────────────────────────── O GUARD (puro) ───────────────────────────────────

/// Régua POR-ARQUIVO (DURA + CATRACA), independente da allowlist global — isolável nas provas de
/// mutação sem o ruído do anti-stale. Dado `(nome, fonte)`, devolve as violações de cada arquivo.
fn per_file_violations(files: &[(String, String)]) -> Vec<String> {
    let mut violations = Vec::new();
    for (name, src) in files {
        // Régua DURA: declarou semântica de anúncio ambiente sem compor o nó → retrofit proibido.
        if declares_ambient_announce(src) && !composes_live_region(src) {
            violations.push(format!(
                "{name}: declara semântica de anúncio AMBIENTE (Role::Status/Alert/Politeness) mas \
                 NÃO compõe `live_region(...)`. ADR 0028: o componente deve NASCER envolvendo o visual \
                 — `live_region(id, msg, politeness, visual)`. Retrofit proibido."
            ));
        }

        // Régua CATRACA: usa cor-de-estado → compõe live_region OU está na allowlist justificada.
        if uses_state_color(src) && !composes_live_region(src) && !is_exempt(name) {
            violations.push(format!(
                "{name}: comunica estado pela cor ({}) mas NÃO compõe `live_region(...)` e não está na \
                 allowlist. Se é STATUS que muda e deve ser falado SEM foco, faça-o NASCER sobre \
                 `a11y_live::live_region` (ADR 0028). Se a cor é de AÇÃO/erro anunciado NO FOCO \
                 (botão/campo), adicione `(\"{name}\", \"<motivo>\")` a FOCUS_STATE_EXEMPTIONS.",
                matched_state_tokens(src)
            ));
        }
    }
    violations
}

/// Anti-stale (a allowlist só desce): toda exceção pinada tem de continuar SE APLICANDO ao fonte —
/// arquivo que sumiu, deixou de usar cor-de-estado, ou graduou a anunciador (compõe) → exceção morta.
fn stale_exemption_violations(files: &[(String, String)]) -> Vec<String> {
    let mut violations = Vec::new();
    for (name, reason) in FOCUS_STATE_EXEMPTIONS {
        match files.iter().find(|(n, _)| n == name) {
            None => violations.push(format!(
                "{name}: exceção aponta para arquivo inexistente em src/ui/ — remova de \
                 FOCUS_STATE_EXEMPTIONS (motivo registrado: {reason})."
            )),
            Some((_, src)) => {
                if composes_live_region(src) {
                    violations.push(format!(
                        "{name}: exceção STALE — o arquivo agora COMPÕE live_region (graduou a \
                         anunciador ambiente). Remova de FOCUS_STATE_EXEMPTIONS."
                    ));
                } else if !uses_state_color(src) {
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

/// O guard COMPLETO (vazio = conforme): régua por-arquivo + anti-stale da allowlist. PURO —
/// alimentado pelo fonte REAL no teste-catraca e por mutantes sintéticos na calibração.
fn scan(files: &[(String, String)]) -> Vec<String> {
    let mut violations = per_file_violations(files);
    violations.extend(stale_exemption_violations(files));
    violations
}

// ─────────────────────────────────── Leitura do fonte real ───────────────────────────────────

fn ui_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui")
}

/// `(nome, fonte)` de cada `*.rs` de `src/ui/` exceto `mod.rs` (registro do módulo, não componente).
fn ui_sources() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(ui_dir()).expect("ler src/ui") {
        let path = entry.expect("entry de src/ui").path();
        let is_rs = path.extension().is_some_and(|e| e == "rs");
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if is_rs && name != "mod.rs" {
            let src = std::fs::read_to_string(&path).expect("ler fonte de ui");
            out.push((name, src));
        }
    }
    out
}

// ─────────────────────────────────────── O teste-guard ───────────────────────────────────────

/// **A catraca viva:** todo componente de `src/ui/` está conforme o ADR 0028. Quebra na CI se um
/// componente novo comunicar estado fora do live-region (ou se uma exceção ficar stale).
#[test]
fn ui_components_conform_to_adr_0028() {
    let violations = scan(&ui_sources());
    assert!(
        violations.is_empty(),
        "Conformidade ADR 0028 (live-region) violada — {} ocorrência(s):\n  - {}",
        violations.len(),
        violations.join("\n  - ")
    );
}

// ─────────────────────────── Calibração por MUTAÇÃO (false-neg / false-pos) ───────────────────────────

/// **False-NEGATIVO guard:** um componente NOVO que comunica estado mas ESQUECE o live-region TEM
/// de cair. Prova por mutação sem tocar o fonte real (mutante sintético).
#[test]
fn mutation_new_state_component_without_live_region_is_caught() {
    let mutant = (
        "badge_novo.rs".to_string(),
        r#"
            pub struct BadgeNovo { label: SharedString }
            impl RenderOnce for BadgeNovo {
                fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
                    let t = theme::active();
                    div().bg(rgb(t.state.success)).child(text!(self.label)) // ESQUECEU o live_region
                }
            }
        "#
        .to_string(),
    );
    let v = per_file_violations(&[mutant]);
    assert!(
        v.iter()
            .any(|m| m.contains("badge_novo.rs") && m.contains("não está na allowlist")),
        "um componente novo de estado sem live_region DEVE ser pego: {v:?}"
    );
}

/// **False-POSITIVO guard:** o MESMO componente, agora compondo o live-region, NÃO cai.
#[test]
fn mutation_compliant_state_component_passes() {
    let ok = (
        "badge_novo.rs".to_string(),
        r#"
            use crate::a11y_live::{live_region, Politeness};
            impl RenderOnce for BadgeNovo {
                fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
                    let t = theme::active();
                    let pill = div().bg(rgb(t.state.success)).child(text!(self.label));
                    live_region(self.id, self.label.to_string(), Politeness::Polite, pill)
                }
            }
        "#
        .to_string(),
    );
    assert!(
        per_file_violations(&[ok]).is_empty(),
        "componente de estado que COMPÕE live_region não pode cair"
    );
}

/// **Régua DURA:** declarar `Role::Alert`/`Politeness` sem compor o nó cai MESMO sem cor-de-estado —
/// usar o papel de live-region sem o Element é o retrofit proibido.
#[test]
fn mutation_ambient_role_without_composition_is_caught_even_without_color() {
    let mutant = (
        "status_pill.rs".to_string(),
        "fn role() -> A11yRole { Role::Alert } // declara alerta, sem live_region nem cor"
            .to_string(),
    );
    let v = per_file_violations(&[mutant]);
    assert!(
        v.iter()
            .any(|m| m.contains("status_pill.rs") && m.contains("AMBIENTE")),
        "papel de anúncio ambiente sem composição DEVE cair: {v:?}"
    );
}

/// **Calibração de substring:** `Role::AlertDialog` (diálogo com foco, ex.: modal) NÃO é
/// `Role::Alert` — a fronteira de palavra evita o falso-positivo.
#[test]
fn alertdialog_role_is_not_an_ambient_announce_marker() {
    assert!(
        !declares_ambient_announce("panel.role(Role::AlertDialog)"),
        "Role::AlertDialog não é Role::Alert (fronteira de palavra)"
    );
    assert!(declares_ambient_announce("Role::Alert,"));
    assert!(!references("state.dangerous_thing", "state.danger"));
}

/// **False-POSITIVO guard:** um componente puramente estrutural (container, sem cor-de-estado nem
/// papel de anúncio) não pode cair.
#[test]
fn structural_component_without_state_is_not_flagged() {
    let panel = (
        "panel.rs".to_string(),
        "div().bg(rgb(t.surface.raised)).p(px(pad)) // só layout".to_string(),
    );
    assert!(
        per_file_violations(&[panel]).is_empty(),
        "container sem estado nem papel de anúncio não cai"
    );
}

/// **Anti-stale:** uma exceção cujo arquivo GRADUOU a anunciador (passou a compor live_region) é
/// reportada para remoção — a allowlist não pode apodrecer.
#[test]
fn stale_exemption_that_now_composes_live_region_is_caught() {
    // `input.rs` está na allowlist; se passar a compor live_region, a exceção fica stale.
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

/// **Anti-stale:** uma exceção cujo arquivo sumiu de `src/ui/` é reportada para remoção.
#[test]
fn exemption_for_missing_file_is_caught() {
    // Nenhum arquivo: as duas exceções pinadas apontam para arquivos ausentes.
    let v = stale_exemption_violations(&[]);
    assert!(
        v.iter()
            .any(|m| m.contains("button.rs") && m.contains("inexistente"))
            && v.iter()
                .any(|m| m.contains("input.rs") && m.contains("inexistente")),
        "exceção apontando para arquivo ausente deve cair: {v:?}"
    );
}

/// **Matchers PINADOS** (anti-enfraquecimento silencioso, como a catraca): a lista de detecção é a
/// esperada; mudar exige tocar este teste — ponto de revisão.
#[test]
fn matchers_are_pinned() {
    assert_eq!(
        STATE_COLOR_TOKENS,
        &["state.success", "state.warning", "state.danger"]
    );
    assert_eq!(
        AMBIENT_ANNOUNCE_MARKERS,
        &["Role::Status", "Role::Alert", "Politeness"]
    );
    assert_eq!(COMPOSE_LIVE_REGION, "live_region(");
    // A allowlist atual: exatamente os dois anunciados-no-foco. Crescer aqui é decisão revisada.
    assert_eq!(
        FOCUS_STATE_EXEMPTIONS
            .iter()
            .map(|(n, _)| *n)
            .collect::<Vec<_>>(),
        vec!["button.rs", "input.rs"]
    );
}
