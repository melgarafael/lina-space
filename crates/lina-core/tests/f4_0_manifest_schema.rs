//! **F4-0-6 (gate e): validação de SCHEMA do manifesto de canal em merge-time.**
//!
//! Curadoria = segurança para o leigo (doc 40 §6/§11): um manifesto de canal MALFORMADO **barra o
//! merge** na CI. Este teste é o validador que o passo de CI (`ci.yml`) roda — `cargo test -p
//! lina-core --test f4_0_manifest_schema`. Dois deveres:
//!
//! 1. **Os manifestos REAIS de `channels/` validam** (todo `channels/<nome>/manifest.toml` que entra
//!    no PR passa `ChannelManifest::load`). Um manifesto malformado commitado de verdade → este teste
//!    FALHA → o job da CI fica vermelho → o merge é barrado. É o gate (e) por evidência, não por fé.
//! 2. **Malformado é rejeitado ANTES de executar** (schema + semântica): campo faltando, tipo errado,
//!    campo desconhecido, `install.ref` flutuante, campo vazio.
//!
//! **Reusa o parser canônico de B** (`ChannelManifest::parse`/`load`, F4-0-1) — uma única fonte da
//! verdade do schema; este teste NÃO redefine o manifesto (seria DRY violado + duas fontes a divergir).
//!
//! **Red-team embutido (manifesto é DADO, jamais autoridade):** um manifesto que tente AUTO-DECLARAR
//! `trust_tier` (prerrogativa da curadoria, não do canal) cai em `deny_unknown_fields` → `Err`. A
//! confiança vem do registro (evento `ChannelRegistered`, server-side), nunca do arquivo do canal.

use std::path::{Path, PathBuf};

use lina_core::channel::{ChannelManifest, ManifestError};

/// Raiz `channels/` do repo (irmã de `crates/`), relativa ao crate `lina-core` em teste.
fn channels_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../channels")
}

/// Manifesto TOML válido mínimo — base de comparação dos casos negativos abaixo.
const VALID_MANIFEST: &str = r#"
name = "demo"
transport = "smtp"
auth = "api_key"
scopes = ["mail:send"]

[tools]
default_enabled = ["send"]

[install]
ref = "v1.0.0"
"#;

/// **Gate (e), dever 1:** todo manifesto REAL em `channels/` valida pelo parser canônico. Se alguém
/// commitar um manifesto malformado, este teste falha — é o que barra o merge na CI.
#[test]
fn manifestos_reais_em_channels_validam() {
    let dir = channels_dir();
    assert!(
        dir.is_dir(),
        "esperava a raiz channels/ em {} (ela é commitada com os manifestos-exemplo)",
        dir.display()
    );
    let mut validados = 0u32;
    for entry in std::fs::read_dir(&dir).expect("ler channels/") {
        let manifest = entry
            .expect("entry de channels/")
            .path()
            .join("manifest.toml");
        if !manifest.is_file() {
            continue;
        }
        validados += 1;
        if let Err(e) = ChannelManifest::load(&manifest) {
            panic!(
                "manifesto real {} deve validar (gate e), mas falhou: {e:?}",
                manifest.display()
            );
        }
    }
    assert!(
        validados > 0,
        "channels/ deve ter ao menos 1 manifesto.toml (exemplos de B)"
    );
}

/// Sanidade do positivo: o manifesto válido mínimo passa (schema + semântica).
#[test]
fn manifesto_valido_minimo_passa() {
    assert!(
        ChannelManifest::parse(VALID_MANIFEST).is_ok(),
        "manifesto válido mínimo deve passar"
    );
}

/// **Gate (e), dever 2 — schema:** campo obrigatório faltando (`auth`) → `Err::Schema`, nunca executa.
#[test]
fn schema_rejeita_campo_obrigatorio_faltando() {
    let toml = r#"
name = "x"
transport = "smtp"

[install]
ref = "v1.0.0"
"#;
    assert!(
        matches!(ChannelManifest::parse(toml), Err(ManifestError::Schema(_))),
        "manifesto sem `auth` deve falhar no schema"
    );
}

/// **Red-team (manifesto não é autoridade):** o canal NÃO pode auto-declarar `trust_tier` — isso é
/// prerrogativa da curadoria (F4-0-6), carimbada no evento `ChannelRegistered`. `deny_unknown_fields`
/// → `Err::Schema`. Um manifesto que tente decidir a própria confiança é barrado antes de executar.
#[test]
fn schema_rejeita_trust_tier_autodeclarado_no_manifesto() {
    let toml = r#"
name = "x"
transport = "smtp"
auth = "api_key"
trust_tier = "core"

[install]
ref = "v1.0.0"
"#;
    assert!(
        matches!(ChannelManifest::parse(toml), Err(ManifestError::Schema(_))),
        "manifesto auto-declarando trust_tier deve falhar (deny_unknown_fields) — curadoria decide, não o canal"
    );
}

/// **Gate (e), dever 2 — semântica:** `install.ref` flutuante (HEAD/main/latest/…) →
/// `Err::FloatingInstallRef`. Uma ref móvel deixaria o instalado mudar sob os pés (doc 40 §6).
#[test]
fn semantica_rejeita_install_ref_flutuante() {
    let toml = r#"
name = "x"
transport = "smtp"
auth = "api_key"

[install]
ref = "HEAD"
"#;
    assert!(
        matches!(
            ChannelManifest::parse(toml),
            Err(ManifestError::FloatingInstallRef(_))
        ),
        "install.ref flutuante deve falhar — só SHA/tag pinada"
    );
}

/// **Gate (e), dever 2 — semântica:** campo obrigatório vazio (`name = ""`) → `Err::EmptyField`.
#[test]
fn semantica_rejeita_campo_vazio() {
    let toml = r#"
name = ""
transport = "smtp"
auth = "api_key"

[install]
ref = "v1.0.0"
"#;
    assert!(
        matches!(
            ChannelManifest::parse(toml),
            Err(ManifestError::EmptyField(_))
        ),
        "campo obrigatório vazio deve falhar"
    );
}
