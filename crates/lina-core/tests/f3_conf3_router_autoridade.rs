//! **F3-CONF-3 · R3 (QA/RED) — a AUTORIDADE do router não regride com o fix de observabilidade.**
//!
//! O invariante que ATRAVESSA a onda (onda-f3-conf-3 §"Invariante"): **observabilidade ≠ autoridade**.
//! A resolução "vivo-vence-morto" do #23c é da superfície de OBSERVABILIDADE (`lina list`/`check`, no
//! bin `lina.rs`). `Supervisor::node_by_name` (`lib.rs:1534`) resolve remetente/alvo do ROUTER
//! (autoridade) e JÁ ignora mortos (`status.is_alive()`, ADR 0007: campo de agente nunca decide
//! identidade). O fix do #23c NÃO toca `lib.rs` — esta suíte é a rede que prova, por MUTAÇÃO, que a
//! autoridade continua correta e desacoplada da observabilidade (gate (e): `node_by_name`/autoridade
//! intactos; 0 ALTA).
//!
//! **Prova por mutação (memória *provar enforcement por mutação*):** o homônimo MORTO é registrado
//! PRIMEIRO. `node_by_name` devolve o 1º vivo na ordem de registro; se a guarda `is_alive()` fosse
//! removida, o morto (primeiro na ordem) venceria. Logo `assert_eq!(resolvido, vivo)` FALHA sem a
//! guarda — não-vacuoso por construção. Cobre o match EXATO e o normalizado (as duas varreduras de
//! `node_by_name`).

use lina_core::Supervisor;

fn sink() -> Box<dyn std::io::Write + Send> {
    Box::new(std::io::sink())
}

/// **A autoridade ignora o morto no match EXATO (morto registrado primeiro = recency/ordem trap).**
/// QUEBRA SE… a guarda `is_alive()` da 1ª varredura de `node_by_name` cair → o morto (1º na ordem)
/// seria devolvido e o `assert_eq!(vivo)` falharia. Não-vacuoso: o nó VIVO de fato resolve.
#[test]
fn node_by_name_ignora_homonimo_morto_match_exato() {
    let sup = Supervisor::new();

    // morto PRIMEIRO (ordem/recência o favoreceria sem a guarda), depois o vivo — mesmo nome.
    let morto = sup.register("@Worker", Some("dev".into()), sink());
    sup.mark_dead(morto).expect("marcar morto");
    let vivo = sup.register("@Worker", Some("dev".into()), sink());

    let resolvido = sup.node_by_name("@Worker");
    assert_eq!(
        resolvido,
        Some(vivo),
        "a autoridade do router resolve o nome para o nó VIVO (ignora o homônimo morto)"
    );
    assert_ne!(
        resolvido,
        Some(morto),
        "o morto registrado primeiro NÃO pode vencer — é a prova de que a guarda is_alive() vale"
    );
}

/// **A autoridade ignora o morto na varredura NORMALIZADA (sigil/caixa).** `@B` morto + `@b` vivo:
/// o exato (`@B`) é morto → pulado; o normalizado (`b`) casa o vivo. QUEBRA SE… a guarda da 2ª
/// varredura cair → o exato morto `@B` venceria. Não-vacuoso: o vivo resolve pelo nome tolerante.
#[test]
fn node_by_name_ignora_morto_no_match_normalizado() {
    let sup = Supervisor::new();

    let morto = sup.register("@B", Some("dev".into()), sink());
    sup.mark_dead(morto).expect("marcar morto");
    let vivo = sup.register("@b", Some("dev".into()), sink());

    // pede com a caixa do MORTO — só a guarda + a tolerância de caixa levam ao vivo.
    let resolvido = sup.node_by_name("@B");
    assert_eq!(
        resolvido,
        Some(vivo),
        "match normalizado (sem @/case-insensitive) resolve o VIVO, nunca o homônimo morto"
    );
}

/// **Sem nenhum vivo, a autoridade resolve para NADA (não promove morto a alvo roteável).** É o
/// fail-safe que impede o router de entregar a um nó morto — a outra metade de "ignora mortos".
/// QUEBRA SE… `node_by_name` devolvesse o morto na ausência de vivo → `Some(_)` em vez de `None`.
#[test]
fn node_by_name_sem_vivo_nao_resolve_para_morto() {
    let sup = Supervisor::new();

    let so_morto = sup.register("@Solo", Some("dev".into()), sink());
    sup.mark_dead(so_morto).expect("marcar morto");

    assert_eq!(
        sup.node_by_name("@Solo"),
        None,
        "nome cujo único nó está morto NÃO resolve (router não roteia para morto)"
    );
}

/// **Contraste (não-vacuosidade global): com o nó VIVO, a autoridade resolve normalmente.** Sem este
/// controle, os testes acima poderiam passar por `node_by_name` estar simplesmente quebrado (sempre
/// `None`). Aqui um nó vivo simples resolve — a função FUNCIONA quando deve.
#[test]
fn node_by_name_resolve_vivo_simples_controle() {
    let sup = Supervisor::new();
    let vivo = sup.register("@Solo", Some("dev".into()), sink());
    assert_eq!(
        sup.node_by_name("@Solo"),
        Some(vivo),
        "controle: nó vivo resolve (a guarda não barra o caminho feliz)"
    );
}
