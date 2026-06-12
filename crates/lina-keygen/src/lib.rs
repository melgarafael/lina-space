//! # lina-keygen — emissão de chaves de licença em lote (F1-4-7)
//!
//! Biblioteca usada pelo binário `lina-keygen` (e pelos testes E2E). Cada
//! chave emitida é ÚNICA e auto-contida — `license_id` UUIDv7 + assinatura
//! ed25519 própria; não existe master key compartilhada: vazar uma chave de
//! aluno não compromete o lote (13.6 ação #8).
//!
//! A chave PRIVADA nunca passa por aqui em forma persistida: o chamador a
//! carrega (de mídia offline/keyring do fundador) e entrega o `SigningKey`.

use ed25519_dalek::{Signer, SigningKey};
use lina_license::{encode_token, LicenseClaims, Verifier};
use time::OffsetDateTime;

/// Uma chave emitida — linha do CSV.
#[derive(Debug, Clone)]
pub struct IssuedKey {
    /// O token que o aluno cola no app (`LINA1.<...>.<...>`).
    pub token: String,
    pub tier: String,
    /// Data de validade ISO (`2027-06-11`) ou `perpetua`.
    pub validade: String,
    pub label: String,
}

/// Parâmetros do lote.
#[derive(Debug, Clone)]
pub struct BatchParams {
    pub count: u32,
    pub tier: String,
    pub label: String,
    /// `None` = perpétua. Em epoch-seconds UTC.
    pub expiry_epoch_s: Option<u64>,
    /// `None` = ilimitado (PRO default).
    pub workspace_limit: Option<u32>,
}

#[derive(Debug, thiserror::Error)]
pub enum KeygenError {
    #[error("campos inválidos para emissão: {0}")]
    InvalidClaims(#[from] lina_license::ClaimsError),
    #[error("falha ao serializar a chave: {0}")]
    Encode(#[from] serde_json::Error),
    #[error("auto-verificação falhou: a chave emitida não valida contra a própria pública")]
    SelfCheckFailed,
    #[error("expiry fora do intervalo representável")]
    ExpiryOutOfRange,
}

/// Gera o lote. Cada chave é auto-verificada contra a pública derivada da
/// própria `signing_key` antes de sair — um lote com chave quebrada nunca
/// chega ao CSV (e ao aluno).
pub fn generate_batch(
    signing_key: &SigningKey,
    params: &BatchParams,
) -> Result<Vec<IssuedKey>, KeygenError> {
    let verifier = Verifier::with_key(signing_key.verifying_key());
    let validade = match params.expiry_epoch_s {
        Some(epoch) => format_iso_date(epoch).ok_or(KeygenError::ExpiryOutOfRange)?,
        None => "perpetua".to_string(),
    };
    let mut out = Vec::with_capacity(params.count as usize);
    for _ in 0..params.count {
        let claims = LicenseClaims {
            // UUIDv7: timestamp + aleatoriedade — única entre execuções,
            // o que garante o critério "duas execuções não repetem chaves".
            license_id: uuid::Uuid::now_v7().to_string(),
            tier: params.tier.clone(),
            workspace_limit: params.workspace_limit,
            entitlements: vec![],
            expiry: params.expiry_epoch_s,
            machine_id: None,
        };
        claims.validate()?;
        let signature = signing_key.sign(&claims.canonical_bytes()).to_bytes();
        if !verifier.verify_claims(&claims, &signature) {
            return Err(KeygenError::SelfCheckFailed);
        }
        out.push(IssuedKey {
            token: encode_token(&claims, &signature)?,
            tier: params.tier.clone(),
            validade: validade.clone(),
            label: params.label.clone(),
        });
    }
    Ok(out)
}

/// Renderiza o CSV (`chave,tier,validade,rotulo`). Os valores são livres de
/// vírgula/linha-nova por construção ([`LicenseClaims::validate`] recusa
/// delimitadores e o token é base64url) — sem necessidade de quoting.
pub fn render_csv(keys: &[IssuedKey]) -> String {
    let mut csv = String::from("chave,tier,validade,rotulo\n");
    for k in keys {
        csv.push_str(&format!(
            "{},{},{},{}\n",
            k.token, k.tier, k.validade, k.label
        ));
    }
    csv
}

/// Converte `--expiry 12m|90d|2y` em epoch-seconds a partir de `now`.
/// Meses/anos são de CALENDÁRIO (12m a partir de 2026-06-11 ⇒ 2027-06-11),
/// com clamp no fim do mês (31/01 + 1m ⇒ 28/02) — o que um aluno espera.
pub fn parse_expiry(spec: &str, now_epoch_s: u64) -> Result<u64, KeygenError> {
    let spec = spec.trim();
    let (num, unit) = spec.split_at(spec.len().saturating_sub(1));
    let n: i64 = num.parse().map_err(|_| KeygenError::ExpiryOutOfRange)?;
    if n <= 0 {
        return Err(KeygenError::ExpiryOutOfRange);
    }
    let now = OffsetDateTime::from_unix_timestamp(
        i64::try_from(now_epoch_s).map_err(|_| KeygenError::ExpiryOutOfRange)?,
    )
    .map_err(|_| KeygenError::ExpiryOutOfRange)?;
    let target = match unit {
        "d" => now.checked_add(time::Duration::days(n)),
        "m" => add_calendar_months(now, n),
        "y" => add_calendar_months(now, n.checked_mul(12).unwrap_or(i64::MAX)),
        _ => return Err(KeygenError::ExpiryOutOfRange),
    }
    .ok_or(KeygenError::ExpiryOutOfRange)?;
    u64::try_from(target.unix_timestamp()).map_err(|_| KeygenError::ExpiryOutOfRange)
}

fn add_calendar_months(dt: OffsetDateTime, months: i64) -> Option<OffsetDateTime> {
    let total = i64::from(dt.month() as u8 - 1).checked_add(months)?;
    let year = i32::try_from(i64::from(dt.year()).checked_add(total.div_euclid(12))?).ok()?;
    let month = time::Month::try_from(u8::try_from(total.rem_euclid(12) + 1).ok()?).ok()?;
    let max_day = month.length(year);
    let day = dt.day().min(max_day);
    let date = time::Date::from_calendar_date(year, month, day).ok()?;
    Some(dt.replace_date(date))
}

fn format_iso_date(epoch_s: u64) -> Option<String> {
    let dt = OffsetDateTime::from_unix_timestamp(i64::try_from(epoch_s).ok()?).ok()?;
    Some(format!(
        "{:04}-{:02}-{:02}",
        dt.year(),
        dt.month() as u8,
        dt.day()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2026-06-11T12:00:00Z
    const NOW: u64 = 1_781_179_200;

    #[test]
    fn expiry_12m_e_calendario_nao_360_dias() {
        let expiry = parse_expiry("12m", NOW).expect("12m");
        assert_eq!(format_iso_date(expiry).expect("data"), "2027-06-11");
    }

    #[test]
    fn expiry_clampa_fim_de_mes() {
        // 2026-01-31 + 1m ⇒ 2026-02-28 (não 03-03).
        let jan31 = 1_769_860_800; // 2026-01-31T12:00:00Z
        let expiry = parse_expiry("1m", jan31).expect("1m");
        assert_eq!(format_iso_date(expiry).expect("data"), "2026-02-28");
    }

    #[test]
    fn expiry_recusa_formatos_invalidos() {
        for spec in ["", "12", "12x", "-3m", "0d", "m"] {
            assert!(parse_expiry(spec, NOW).is_err(), "{spec:?} deveria falhar");
        }
    }
}
