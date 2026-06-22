//! **F3-4 · Gate (e2) — `paths`/`author_node` é DADO, jamais autoridade (QA/Red-team).**
//!
//! A regra-mãe que ATRAVESSA a onda (onda-f3-4 §"Invariante"): `paths`/`branch`/`author_node`/
//! `commit` é DADO transportado, JAMAIS autoridade. A comparação `paths`×claims só pode **abrir um
//! item de atenção** (fail-safe restritivo: PARA e pergunta), NUNCA conceder posse nem autorizar.
//!
//! Esta suíte prova o ALICERCE testável-sem-fiação, num ângulo NOVO (não re-prova o já coberto —
//! `f3_conf3_router_autoridade` cobre `node_by_name`; `plan.rs` cobre `AlreadyOwned` internamente):
//! **um `CodeChanged` — o sinal de mudança que um agente emite — NUNCA altera a posse no plano**,
//! porque o evento é META (no-op na projeção, `events.rs:1778`). Um agente não compra posse nem a
//! transfere escrevendo `author_node`/`paths` num sinal: a posse segue o `owner` validado.
//!
//! O lado que depende da Trilha B (o handler que, na interseção `paths`×claim, ABRE atenção
//! `CodeConflict` — e o broadcast por pertencimento que respeita só vivos) é alvo diferencial,
//! ligado na integração — ver `f3_4_alvos_diferenciais.md`.

use lina_core::{apply, DomainEvent, PlanError, ProjectedState};

/// **(e2) Um sinal de código NÃO concede nem transfere posse — `author_node`/`paths` é dado.**
/// Monto um plano onde `T1` é de `@Dona`. Duas tentativas de roubo:
///   (1) reivindicação DIRETA por outro nome → barrada por `owner` validado (`AlreadyOwned`);
///   (2) um `CodeChanged` com `author_node` de um intruso e `paths` que intersecta o arquivo de
///       `T1` → META/no-op: a projeção (logo a posse) fica intacta.
/// O sinal de mudança, no máximo, abre um alerta (handler da Trilha B); jamais decide autoridade.
///
/// Não-vacuoso: o sub-assert (1) prova que a posse de fato RESISTE a outro nome (a guarda de owner
/// existe e morde); o sub-assert (2) prova que o evento de código não é um atalho para furá-la.
/// QUEBRA SE… `CodeChanged` saísse da lista no-op de `apply` e passasse a mexer no `plan` projetado
/// → o owner de `T1` mudaria e o `assert_eq!` falharia (= sinal de agente virou autoridade).
#[test]
fn sinal_de_codigo_nunca_altera_posse() {
    let arquivo = "crates/lina-core/src/router.rs";

    let mut estado = ProjectedState::default();
    estado
        .plan
        .add_item("T1", "mexer no router")
        .expect("add T1");
    estado
        .plan
        .try_claim("T1", "@Dona")
        .expect("@Dona reivindica T1");

    // (1) controle/não-vacuoso: posse vem do owner validado — outro nome é barrado.
    let roubo_direto = estado.plan.try_claim("T1", "@Intruso");
    assert!(
        matches!(roubo_direto, Err(PlanError::AlreadyOwned { .. })),
        "a posse de T1 segue o owner validado (@Dona) — reivindicação por @Intruso é barrada"
    );

    let posse_antes = estado.plan.clone();

    // (2) alvo: um CodeChanged de um intruso, tocando o arquivo de T1, NÃO transfere nada.
    apply(
        &mut estado,
        &DomainEvent::CodeChanged {
            branch: "lina/intruso".to_string(),
            paths: vec![arquivo.to_string()],
            author_node: "@Intruso".to_string(),
            commit: "0bada55".to_string(),
        },
    );

    assert_eq!(
        estado.plan, posse_antes,
        "o sinal de código é META: NÃO altera o plano projetado (posse não é decidida por author_node/paths)"
    );
    // reforço legível (só API pública): @Dona ainda reivindica T1 sem conflito (claim idempotente
    // para o mesmo dono) → continua dona. Se o CodeChanged tivesse transferido a @Intruso, isto
    // seria `AlreadyOwned`.
    assert!(
        estado.plan.try_claim("T1", "@Dona").is_ok(),
        "T1 continua de @Dona após o CodeChanged do intruso — author_node/paths é dado, jamais autoridade"
    );
}
