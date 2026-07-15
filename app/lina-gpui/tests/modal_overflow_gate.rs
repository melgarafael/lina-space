//! **Gate de qualidade — contenção de overflow em modais (skill `lina-modal-doctrine`).**
//!
//! Recusa a anti-marca estrutural que quebrou o modal Criar Espaço (tela do fundador,
//! 2026-06-26): um overlay `.absolute()` que **rola a si mesmo** (`.overflow_y_scroll()` no
//! próprio painel). Num `flex_col` com teto, isso faz o taffy ENCOLHER os filhos ("comidos")
//! em vez de transbordar — e nada rola. O padrão correto separa papéis: o painel CLIPA
//! (`.overflow_hidden()`) e o scroll mora num **corpo-filho** com `min_h(px(0.))` (ou viewport
//! com `max_h`). Ver `ui::modal::Modal` (o componente que já encoda a doutrina) e a skill.
//!
//! **Como detecta (sem parser de verdade, mas sem falso-positivo de aninhamento):** divide a
//! fonte por `.child(` / `.children(` / `;` — cada fragmento ≈ os métodos de UM elemento (os
//! métodos vêm antes dos filhos no builder gpui). Um fragmento com `.absolute()` E um scroll é
//! o mesmo elemento rolando a si mesmo → viola. O scroll de um corpo-filho cai em OUTRO
//! fragmento (depois do `.child(` do pai), então o padrão correto passa limpo.
//!
//! Limite honesto (defesa em profundidade, não bala de prata): não pega o caso raro de método
//! de scroll interleavado DEPOIS de um `.child(` no mesmo elemento, nem a falta de `min_h(0)`
//! num corpo legítimo. Esses ficam por conta da doutrina + do uso de `ui::Modal`.

use std::path::{Path, PathBuf};

/// Todos os `.rs` sob `src/` (recursivo).
fn shell_sources(src: &Path) -> Vec<PathBuf> {
    let mut pending = vec![src.to_path_buf()];
    let mut sources = Vec::new();
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).expect("ler diretório de fontes do shell") {
            let path = entry.expect("entry de src/").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                sources.push(path);
            }
        }
    }
    sources
}

/// Divide em fragmentos por `.child(` / `.children(` / `;` — cada um ≈ um elemento.
fn fragments(src: &str) -> Vec<&str> {
    src.split(';')
        .flat_map(|s| s.split(".children("))
        .flat_map(|s| s.split(".child("))
        .collect()
}

/// Um fragmento viola se o MESMO elemento é `.absolute()` e rola a si mesmo.
fn fragment_violates(frag: &str) -> bool {
    frag.contains(".absolute()")
        && (frag.contains(".overflow_y_scroll()") || frag.contains(".overflow_x_scroll()"))
}

/// Linha aproximada da 1ª ocorrência de scroll no arquivo (para o relatório).
fn first_scroll_line(src: &str) -> usize {
    src.lines()
        .enumerate()
        .find(|(_, l)| l.contains(".overflow_y_scroll()") || l.contains(".overflow_x_scroll()"))
        .map_or(0, |(i, _)| i + 1)
}

#[test]
fn nenhum_overlay_absolute_rola_a_si_mesmo() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut violacoes = Vec::new();
    for path in shell_sources(&manifest.join("src")) {
        let content = std::fs::read_to_string(&path).expect("ler fonte do shell");
        if fragments(&content).iter().any(|f| fragment_violates(f)) {
            let rel = path
                .strip_prefix(manifest)
                .unwrap_or(&path)
                .display()
                .to_string();
            violacoes.push(format!(
                "  {rel} (perto da linha {})",
                first_scroll_line(&content)
            ));
        }
    }
    assert!(
        violacoes.is_empty(),
        "Modal/painel viola a doutrina de overflow (skill lina-modal-doctrine): um overlay \
         .absolute() está rolando a si mesmo. O painel deve CLIPAR (.overflow_hidden()) e o \
         scroll mora num corpo-filho com .min_h(px(0.)) (ou viewport com .max_h). Prefira o \
         componente ui::modal::Modal, que já resolve.\nArquivos:\n{}",
        violacoes.join("\n")
    );
}

/// **Não-vácuo (prova por mutação):** o detector ACUSA a anti-marca real e ABSOLVE o padrão
/// correto. Sem isto, o gate poderia passar por estar cego, não por estar limpo.
#[test]
fn detector_acusa_anti_marca_e_absolve_padrao_correto() {
    // Anti-marca: o painel .absolute() rola a si mesmo (o bug do m9-panel original).
    let ruim = r#"div().id("p").absolute().max_h(px(h)).overflow_y_scroll().flex().flex_col()"#;
    assert!(
        fragments(ruim).iter().any(|f| fragment_violates(f)),
        "deveria acusar o overlay que rola a si mesmo"
    );

    // Correto: painel .absolute() CLIPA; o scroll vive num corpo-filho (.child(...)).
    let bom = r#"div().id("p").absolute().max_h(px(h)).overflow_hidden().flex().flex_col()
        .child(div().id("body").flex_1().min_h(px(0.)).overflow_y_scroll().child(rows))"#;
    assert!(
        !fragments(bom).iter().any(|f| fragment_violates(f)),
        "o padrão correto (scroll no corpo-filho) NÃO pode ser acusado"
    );
}
