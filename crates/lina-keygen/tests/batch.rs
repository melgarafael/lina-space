//! Provas dos critérios de F1-4-7, rodando o BINÁRIO real (`CARGO_BIN_EXE`)
//! e validando as chaves emitidas contra o `lina-license` — o E2E offline
//! completo: emitir → CSV → ativar no produto.

use std::path::{Path, PathBuf};
use std::process::Command;

use lina_license::{activate, ActivationError, LicenseState, LicenseStatus, Verifier};

const NOW: u64 = 1_781_179_200; // 2026-06-11T12:00:00Z

fn keygen_bin() -> &'static str {
    env!("CARGO_BIN_EXE_lina-keygen")
}

fn run(args: &[&str], cwd: &Path) -> std::process::Output {
    Command::new(keygen_bin())
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("executar lina-keygen")
}

/// Gera keypair + lote num tempdir; devolve (verifier, linhas de chave do CSV).
fn issue_batch(dir: &Path, count: &str, csv_name: &str) -> (Verifier, Vec<String>) {
    let keys_dir = dir.join("segredo");
    let out = run(&["keypair", "--out", keys_dir.to_str().expect("utf8")], dir);
    assert!(
        out.status.success(),
        "keypair falhou: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let csv_path = dir.join(csv_name);
    let out = run(
        &[
            "gen",
            "--count",
            count,
            "--tier",
            "pro",
            "--label",
            "turma-7",
            "--expiry",
            "12m",
            "--private-key",
            keys_dir
                .join("lina-signing.private")
                .to_str()
                .expect("utf8"),
            "--out",
            csv_path.to_str().expect("utf8"),
        ],
        dir,
    );
    assert!(
        out.status.success(),
        "gen falhou: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let public_hex =
        std::fs::read_to_string(keys_dir.join("lina-signing.public")).expect("ler pública");
    let verifier = Verifier::from_hex(public_hex.trim()).expect("pública válida");

    let csv = std::fs::read_to_string(&csv_path).expect("ler CSV");
    let mut lines = csv.lines();
    assert_eq!(
        lines.next(),
        Some("chave,tier,validade,rotulo"),
        "header do CSV"
    );
    let tokens: Vec<String> = lines
        .map(|l| {
            let mut cols = l.split(',');
            let token = cols.next().expect("coluna chave").to_string();
            assert_eq!(cols.clone().count(), 3, "linha com 4 colunas: {l}");
            assert!(token.starts_with("LINA1."), "token com prefixo: {token}");
            token
        })
        .collect();
    (verifier, tokens)
}

// Critério 1: 50 chaves distintas; duas execuções não repetem chaves.
#[test]
fn cinquenta_chaves_distintas_e_execucoes_nao_repetem() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (_, lote_a) = issue_batch(dir.path(), "50", "a.csv");
    assert_eq!(lote_a.len(), 50);
    let unicas: std::collections::HashSet<_> = lote_a.iter().collect();
    assert_eq!(unicas.len(), 50, "todas as 50 chaves são distintas");

    let dir2 = tempfile::tempdir().expect("tempdir");
    let (_, lote_b) = issue_batch(dir2.path(), "50", "b.csv");
    assert!(
        lote_a.iter().all(|t| !lote_b.contains(t)),
        "nenhuma chave se repete entre execuções"
    );
}

// Critério 2 (E2E offline): 3 chaves amostradas do CSV ativam no lina-license.
#[test]
fn tres_chaves_amostradas_ativam_offline_no_produto() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (verifier, tokens) = issue_batch(dir.path(), "10", "lote.csv");
    for (i, token) in [&tokens[0], &tokens[4], &tokens[9]].iter().enumerate() {
        let license_path: PathBuf = dir.path().join(format!("license-{i}.json"));
        let claims =
            activate(token, &license_path, &verifier, NOW).expect("chave do CSV ativa offline");
        assert_eq!(claims.tier, "pro");
        let state = LicenseState::load(&license_path, &verifier);
        assert_eq!(*state.status(), LicenseStatus::Licensed);
    }
}

// Critério 3: chave adulterada em 1 byte falha na validação.
#[test]
fn chave_adulterada_um_byte_falha() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (verifier, tokens) = issue_batch(dir.path(), "1", "lote.csv");
    let token = &tokens[0];

    // Flipa 1 caractere no meio do payload (entre o 1º e o 2º ponto).
    let dot = token.find('.').expect("tem ponto") + 10;
    let mut chars: Vec<char> = token.chars().collect();
    chars[dot] = if chars[dot] == 'A' { 'B' } else { 'A' };
    let adulterada: String = chars.into_iter().collect();
    assert_ne!(&adulterada, token);

    let err = activate(
        &adulterada,
        &dir.path().join("license.json"),
        &verifier,
        NOW,
    )
    .expect_err("adulterada não ativa");
    assert!(matches!(
        err,
        ActivationError::InvalidSignature | ActivationError::MalformedKey(_)
    ));
}

// Governança: a privada nasce com permissão restrita e fora do CSV.
#[test]
fn privada_nasce_restrita_e_nao_vaza_no_csv() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (_, _tokens) = issue_batch(dir.path(), "5", "lote.csv");
    let private_path = dir.path().join("segredo").join("lina-signing.private");
    let private_hex = std::fs::read_to_string(&private_path).expect("ler privada");
    let csv = std::fs::read_to_string(dir.path().join("lote.csv")).expect("ler CSV");
    assert!(
        !csv.contains(private_hex.trim()),
        "privada jamais aparece no CSV"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&private_path)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "privada é 0600");
    }

    // `keypair` se recusa a sobrescrever uma privada existente.
    let out = run(
        &[
            "keypair",
            "--out",
            dir.path().join("segredo").to_str().expect("utf8"),
        ],
        dir.path(),
    );
    assert!(!out.status.success(), "não sobrescreve privada existente");
}
