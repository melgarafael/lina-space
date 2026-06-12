//! **F2-1-5 — teste-catraca de tokens:** a dívida de magic numbers do shell só pode CAIR.
//!
//! Modelado no lint de cor `no_hardcoded_colors_outside_theme_module` (theme.rs, F1-2-1):
//! varre `src/**/*.rs` em tempo de teste e compara a contagem POR ARQUIVO contra o snapshot
//! versionado `tests/token_ratchet_snapshot.txt`.
//!
//! Categorias contadas (fora de `theme.rs`, a fonte dos tokens):
//! - `px(<literal numérico ≠ 0>)` — espaçamento/medida mágica. Consumo de token é
//!   `px(f32::from(t.spacing.md))` (doc de `SpacingTokens`) e NÃO conta: o argumento não é
//!   literal. `px(0.)`/`px(0.0)` não contam — reset estrutural (`min_w(px(0.))` do flexbox),
//!   não decisão de design; a exclusão é por VALOR parseado, então `px(0.5)` (hairline) conta.
//! - `FontWeight::` — peso de fonte por constante gpui em vez de `t.typography.weight.*`.
//! - `.text_size(px(<literal numérico>))` — tamanho de texto fora da escala tipográfica.
//!   O despacho pedia o padrão cru `.text_size(px(`, mas o consumo CORRETO do token também
//!   casa com ele (`.text_size(px(f32::from(t.typography.size.body)))`) — contar o cru
//!   puniria a migração certa; o discriminador é o literal dentro do `px(`.
//!   (Sobreposição deliberada: um `.text_size(px(11.))` conta aqui E em `px(<literal>)` —
//!   são réguas independentes da mesma linha.)
//!
//! Semântica da catraca (qualquer divergência do snapshot FALHA, com direção):
//! - contagem SUBIU → regressão: use os tokens; nunca edite o snapshot para cima.
//! - contagem CAIU → aperte a catraca: `LINA_RATCHET_UPDATE=1 cargo test -p lina-gpui
//!   --test token_ratchet` regenera o snapshot; commite-o junto da limpeza.
//!
//! Código de `#[cfg(test)]` dentro de `src/` conta como dívida igual (hoje: zero ocorrências;
//! manter a regra mecânica evita um furo de parsing de cfg).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const SNAPSHOT_REL: &str = "tests/token_ratchet_snapshot.txt";
const UPDATE_ENV: &str = "LINA_RATCHET_UPDATE";

/// Dívida de uma fonte do shell, nas 3 categorias da catraca (ordem = colunas do snapshot).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct FileDebt {
    px_literal: usize,
    font_weight: usize,
    text_size_px: usize,
}

impl FileDebt {
    fn is_clean(&self) -> bool {
        *self == Self::default()
    }

    fn by_category(&self) -> [(&'static str, usize); 3] {
        [
            ("px(<literal>)", self.px_literal),
            ("FontWeight::", self.font_weight),
            (".text_size(px(<literal>))", self.text_size_px),
        ]
    }
}

/// Lê o literal numérico logo após um `px(` (a partir de `rest`, já sem o prefixo).
/// `None` = argumento não é literal (variável/expressão — consumo de token, não conta).
fn numeric_literal_value(rest: &str) -> Option<f64> {
    let rest = rest.trim_start();
    let unsigned = rest.strip_prefix('-').unwrap_or(rest);
    if !unsigned.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let len = unsigned
        .find(|c: char| !c.is_ascii_digit() && c != '.' && c != '_')
        .unwrap_or(unsigned.len());
    unsigned[..len].replace('_', "").parse::<f64>().ok()
}

/// Ocorrências de `needle` em `line` cuja continuação é um literal numérico; com
/// `nonzero_only`, literais de valor 0 ficam de fora (reset estrutural — doc do módulo).
fn count_literal_calls(line: &str, needle: &str, nonzero_only: bool) -> usize {
    let mut count = 0;
    for (i, _) in line.match_indices(needle) {
        // Fronteira de palavra: `col_px(`/`row_px(` (helpers de teste em main.rs) não são `px(`.
        let preceded_by_ident = line[..i]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
        if preceded_by_ident {
            continue;
        }
        match numeric_literal_value(&line[i + needle.len()..]) {
            Some(value) if !(nonzero_only && value == 0.0) => count += 1,
            _ => {}
        }
    }
    count
}

fn debt_of_source(content: &str) -> FileDebt {
    let mut debt = FileDebt::default();
    for line in content.lines() {
        debt.px_literal += count_literal_calls(line, "px(", true);
        debt.font_weight += line.matches("FontWeight::").count();
        debt.text_size_px += count_literal_calls(line, ".text_size(px(", false);
    }
    debt
}

/// Todos os `.rs` sob `src/` (recursivo — o lint de cor não recursa, mas uma catraca com
/// ponto cego em subdiretório futuro deixaria dívida subir em silêncio), exceto `theme.rs`.
fn shell_sources(src: &Path) -> Vec<PathBuf> {
    let mut pending = vec![src.to_path_buf()];
    let mut sources = Vec::new();
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).expect("ler diretório de fontes do shell") {
            let path = entry.expect("entry de src/").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|e| e == "rs")
                && path.file_name().is_some_and(|n| n != "theme.rs")
            {
                sources.push(path);
            }
        }
    }
    sources
}

fn scan_shell_debt() -> BTreeMap<String, FileDebt> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut per_file = BTreeMap::new();
    for path in shell_sources(&manifest.join("src")) {
        let content = std::fs::read_to_string(&path).expect("ler fonte do shell");
        let debt = debt_of_source(&content);
        if !debt.is_clean() {
            let name = path
                .file_name()
                .expect("fonte tem nome")
                .to_string_lossy()
                .into_owned();
            per_file.insert(name, debt);
        }
    }
    per_file
}

fn snapshot_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(SNAPSHOT_REL)
}

fn write_snapshot(per_file: &BTreeMap<String, FileDebt>) {
    let mut out = String::from(
        "# Catraca de tokens (F2-1-5) — dívida de magic numbers POR ARQUIVO; só pode CAIR.\n\
         # Gerado por tests/token_ratchet.rs; regenerar APENAS após queda real:\n\
         #   LINA_RATCHET_UPDATE=1 cargo test -p lina-gpui --test token_ratchet\n\
         # arquivo px_literal font_weight text_size_px\n",
    );
    for (name, debt) in per_file {
        writeln!(
            out,
            "{name} {} {} {}",
            debt.px_literal, debt.font_weight, debt.text_size_px
        )
        .expect("escrever em String não falha");
    }
    std::fs::write(snapshot_path(), out).expect("escrever snapshot da catraca");
}

fn read_snapshot() -> BTreeMap<String, FileDebt> {
    let path = snapshot_path();
    let content = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "snapshot da catraca ausente/ilegível ({}): {e}\n\
             bootstrap: LINA_RATCHET_UPDATE=1 cargo test -p lina-gpui --test token_ratchet \
             (e commite o arquivo)",
            path.display()
        )
    });
    let mut per_file = BTreeMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split_whitespace();
        let parse = |f: Option<&str>| {
            f.and_then(|v| v.parse::<usize>().ok())
                .unwrap_or_else(|| panic!("linha malformada no snapshot da catraca: `{line}`"))
        };
        let name = fields
            .next()
            .expect("split_whitespace nunca é vazio")
            .to_owned();
        per_file.insert(
            name,
            FileDebt {
                px_literal: parse(fields.next()),
                font_weight: parse(fields.next()),
                text_size_px: parse(fields.next()),
            },
        );
    }
    per_file
}

/// **O teste-catraca.** Compara a dívida atual com o snapshot e falha em QUALQUER divergência
/// — subir é regressão; cair exige apertar a catraca (atualizar o snapshot) no mesmo commit.
#[test]
fn magic_number_debt_only_goes_down() {
    let current = scan_shell_debt();
    if std::env::var(UPDATE_ENV).is_ok_and(|v| v == "1") {
        write_snapshot(&current);
        return;
    }
    let snapshot = read_snapshot();

    let mut regressions = Vec::new();
    let mut tightenings = Vec::new();
    let files: std::collections::BTreeSet<&String> =
        current.keys().chain(snapshot.keys()).collect();
    for file in files {
        let cur = current.get(file).copied().unwrap_or_default();
        let snap = snapshot.get(file).copied().unwrap_or_default();
        for ((category, cur_n), (_, snap_n)) in cur.by_category().iter().zip(snap.by_category()) {
            match cur_n.cmp(&snap_n) {
                std::cmp::Ordering::Greater => {
                    regressions.push(format!("{file}: {category} subiu {snap_n} → {cur_n}"))
                }
                std::cmp::Ordering::Less => {
                    tightenings.push(format!("{file}: {category} caiu {snap_n} → {cur_n}"))
                }
                std::cmp::Ordering::Equal => {}
            }
        }
    }

    assert!(
        regressions.is_empty(),
        "DÍVIDA DE MAGIC NUMBERS SUBIU — a catraca não sobe. Use os tokens do tema \
         (`theme::active()`): espaçamento/medida = `px(f32::from(t.spacing.*))`, peso = \
         `t.typography.weight.*`, tamanho de texto = `t.typography.size.*`. NÃO edite o \
         snapshot para acomodar regressão.\n{}",
        regressions.join("\n")
    );
    assert!(
        tightenings.is_empty(),
        "Dívida CAIU (boa!) — aperte a catraca para ela não voltar: rode \
         `LINA_RATCHET_UPDATE=1 cargo test -p lina-gpui --test token_ratchet` e commite \
         tests/token_ratchet_snapshot.txt junto da limpeza.\n{}",
        tightenings.join("\n")
    );
}

/// Pina o comportamento dos matchers — uma regressão silenciosa aqui (ex.: contador que
/// devolve 0 para tudo) zeraria a catraca e um `LINA_RATCHET_UPDATE=1` distraído a selaria.
#[test]
fn matchers_detect_and_ignore_known_patterns() {
    let debt = |s: &str| debt_of_source(s);

    // px(<literal>) conta; consumo de token, helpers *_px e zeros estruturais NÃO.
    assert_eq!(debt(".w(px(260.)).h(px(44.))").px_literal, 2);
    assert_eq!(debt("px(-4.)").px_literal, 1);
    assert_eq!(
        debt("px(0.5)").px_literal,
        1,
        "hairline 0.5 é dívida, não reset"
    );
    assert_eq!(
        debt(".min_w(px(0.)).min_h(px(0.0))").px_literal,
        0,
        "reset flex"
    );
    assert_eq!(debt("px(f32::from(t.spacing.md))").px_literal, 0, "token");
    assert_eq!(
        debt("px(CARD_W)").px_literal,
        0,
        "constante nomeada não é literal cru"
    );
    assert_eq!(
        debt("col_px(10.0) + row_px(2.0)").px_literal,
        0,
        "fronteira de palavra"
    );

    // FontWeight:: conta constante inline; construção via token não menciona `::`.
    assert_eq!(debt(".font_weight(FontWeight::BOLD)").font_weight, 1);
    assert_eq!(
        debt("FontWeight(f32::from(t.typography.weight.bold))").font_weight,
        0
    );

    // .text_size(px(...)): literal conta, token não — e a linha também conta em px_literal.
    let inline = debt(".text_size(px(11.))");
    assert_eq!((inline.text_size_px, inline.px_literal), (1, 1));
    let tokened = debt(".text_size(px(f32::from(t.typography.size.body)))");
    assert_eq!((tokened.text_size_px, tokened.px_literal), (0, 0));
}
