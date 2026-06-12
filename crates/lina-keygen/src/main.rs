//! Binário `lina-keygen` — ferramenta de emissão do FUNDADOR (F1-4-7).
//! Roda 100% offline, na máquina do fundador. Nunca entra no bundle do app.
//! Operação completa (guarda da privada, rotação): ver `OPERACAO.md`.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use ed25519_dalek::SigningKey;
use lina_keygen::{generate_batch, parse_expiry, render_csv, BatchParams};

#[derive(Parser)]
#[command(
    name = "lina-keygen",
    about = "Emissão offline de chaves de licença do Lina Space"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Gera um novo par de chaves (faça UMA vez; guarde a privada offline).
    Keypair {
        /// Diretório onde gravar `lina-signing.private` e `lina-signing.public`.
        #[arg(long)]
        out: PathBuf,
    },
    /// Emite um lote de chaves de licença em CSV.
    Gen {
        /// Quantidade de chaves (ex.: 50).
        #[arg(long)]
        count: u32,
        /// Plano embutido nas chaves (ex.: pro).
        #[arg(long)]
        tier: String,
        /// Rótulo do lote (ex.: turma-7) — vai no CSV para rastrear a origem.
        #[arg(long)]
        label: String,
        /// Validade: `12m`, `90d`, `1y` (meses/anos de calendário). Omitido = perpétua.
        #[arg(long)]
        expiry: Option<String>,
        /// Limite de workspaces por chave. Omitido = ilimitado.
        #[arg(long)]
        workspace_limit: Option<u32>,
        /// Arquivo da chave privada (`lina-signing.private`).
        #[arg(long)]
        private_key: PathBuf,
        /// CSV de saída. Padrão: `chaves-<label>.csv` no diretório atual.
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Keypair { out } => keypair(&out),
        Cmd::Gen {
            count,
            tier,
            label,
            expiry,
            workspace_limit,
            private_key,
            out,
        } => gen(
            count,
            tier,
            label,
            expiry.as_deref(),
            workspace_limit,
            &private_key,
            out,
        ),
    }
}

fn keypair(out_dir: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(out_dir)
        .with_context(|| format!("criando diretório {}", out_dir.display()))?;
    let private_path = out_dir.join("lina-signing.private");
    if private_path.exists() {
        bail!(
            "{} já existe — não vou sobrescrever uma chave privada. \
             Mova-a antes, ou escolha outro --out.",
            private_path.display()
        );
    }
    let key = SigningKey::generate(&mut rand_core::OsRng);
    let private_hex = hex(&key.to_bytes());
    let public_hex = hex(key.verifying_key().as_bytes());

    std::fs::write(&private_path, format!("{private_hex}\n"))
        .with_context(|| format!("gravando {}", private_path.display()))?;
    restrict_permissions(&private_path)?;
    let public_path = out_dir.join("lina-signing.public");
    std::fs::write(&public_path, format!("{public_hex}\n"))
        .with_context(|| format!("gravando {}", public_path.display()))?;

    println!("Par de chaves criado.");
    println!(
        "  privada : {}  (NUNCA entra no repo/bundle — guarde offline)",
        private_path.display()
    );
    println!("  pública : {}", public_path.display());
    println!();
    println!("Para o app reconhecer suas chaves, embarque a pública no binário:");
    println!("  crates/lina-license/src/token.rs → OFFICIAL_PUBLIC_KEY_HEX = \"{public_hex}\"");
    println!("Detalhes e rotação: crates/lina-keygen/OPERACAO.md");
    Ok(())
}

fn gen(
    count: u32,
    tier: String,
    label: String,
    expiry: Option<&str>,
    workspace_limit: Option<u32>,
    private_key: &std::path::Path,
    out: Option<PathBuf>,
) -> Result<()> {
    if count == 0 {
        bail!("--count precisa ser ≥ 1");
    }
    let key = load_signing_key(private_key)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("relógio do sistema antes de 1970")?
        .as_secs();
    let expiry_epoch_s = expiry
        .map(|spec| {
            parse_expiry(spec, now)
                .with_context(|| format!("--expiry `{spec}` inválido (use ex.: 12m, 90d, 1y)"))
        })
        .transpose()?;
    let batch = generate_batch(
        &key,
        &BatchParams {
            count,
            tier,
            label: label.clone(),
            expiry_epoch_s,
            workspace_limit,
        },
    )?;
    let out = out.unwrap_or_else(|| PathBuf::from(format!("chaves-{label}.csv")));
    std::fs::write(&out, render_csv(&batch))
        .with_context(|| format!("gravando {}", out.display()))?;
    println!("{count} chaves emitidas em {}", out.display());
    Ok(())
}

fn load_signing_key(path: &std::path::Path) -> Result<SigningKey> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("lendo chave privada em {}", path.display()))?;
    let raw = raw.trim();
    if raw.len() != 64 || !raw.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!(
            "{} não contém uma chave privada válida (64 hex)",
            path.display()
        );
    }
    let mut bytes = [0u8; 32];
    for (i, chunk) in raw.as_bytes().chunks(2).enumerate() {
        let s = std::str::from_utf8(chunk).context("hex inválido")?;
        bytes[i] = u8::from_str_radix(s, 16).context("hex inválido")?;
    }
    Ok(SigningKey::from_bytes(&bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A privada nasce só-leitura-do-dono (0600). No Windows o ACL herdado do
/// perfil do usuário já restringe; não há equivalente simples portátil.
fn restrict_permissions(path: &std::path::Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("restringindo permissões de {}", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}
