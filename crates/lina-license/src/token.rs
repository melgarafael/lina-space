//! O token de chave de licença — a string que o usuário cola (F1-4-6) e que o
//! `lina-keygen` emite no CSV (F1-4-7).
//!
//! Formato: `LINA1.<base64url(claims JSON)>.<base64url(assinatura ed25519)>`.
//! Auto-contido: a assinatura cobre os bytes canônicos derivados dos claims
//! (ver [`crate::claims`]), então o token carrega tudo que a validação precisa
//! — zero rede, zero servidor de contas (invariante #2, 13.6 ações #2/#8).

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ed25519_dalek::{Signature, VerifyingKey};

use crate::claims::LicenseClaims;

/// Prefixo humano do token (facilita suporte: "sua chave começa com LINA1?").
pub const TOKEN_PREFIX: &str = "LINA1";

/// Chave pública OFICIAL do Lina Space, embarcada no binário (13.6 ação #3).
/// A privada correspondente vive APENAS na máquina do fundador (keyring/mídia
/// offline — ver `crates/lina-keygen/OPERACAO.md`); nunca no repo nem no bundle.
/// Rotação = gerar novo par no `lina-keygen`, trocar ESTA constante e rebuildar.
pub const OFFICIAL_PUBLIC_KEY_HEX: &str =
    "3be6d1f32924a4436e8176facbebdaad1d42bb7bce399c0aaa13dcb87f07ac60";

/// Erros de ativação — variantes pensadas para a UI de F1-4-6 traduzir em
/// copy leiga acionável (cada variante é um caso de mensagem distinto).
#[derive(Debug, thiserror::Error)]
pub enum ActivationError {
    /// A string não tem a forma de uma chave Lina (truncada, prefixo errado).
    /// Copy sugerida: "essa chave está incompleta — confira o e-mail de compra".
    #[error("chave malformada: {0}")]
    MalformedKey(String),
    /// Forma ok, mas a assinatura não bate com a chave pública oficial
    /// (chave de terceiro, adulterada, ou claims editados).
    #[error("assinatura da chave inválida")]
    InvalidSignature,
    /// Chave autêntica porém vencida no momento da ativação.
    #[error("chave expirada em {expiry} (epoch s)")]
    Expired { expiry: u64 },
    /// Falha ao persistir o license.json (disco/permissão).
    #[error("falha ao gravar a licença: {0}")]
    Io(#[from] std::io::Error),
}

/// Verificador de assinaturas. Em produção use [`Verifier::official`]; testes
/// e o E2E do keygen injetam chaves próprias via [`Verifier::with_key`].
///
/// `key = None` é o estado "constante embarcada corrompida": toda verificação
/// devolve `false` e o app degrada para free — nunca panica (regra comum 6).
#[derive(Debug, Clone)]
pub struct Verifier {
    key: Option<VerifyingKey>,
}

impl Verifier {
    /// Verificador com a chave pública oficial embarcada no binário.
    pub fn official() -> Self {
        Self::from_hex(OFFICIAL_PUBLIC_KEY_HEX).unwrap_or(Self { key: None })
    }

    /// Verificador com chave pública explícita (testes, keygen E2E).
    pub fn with_key(key: VerifyingKey) -> Self {
        Self { key: Some(key) }
    }

    /// Constrói a partir de hex de 32 bytes (formato da constante embarcada
    /// e do `public.key` emitido pelo `lina-keygen keypair`).
    pub fn from_hex(hex: &str) -> Result<Self, ActivationError> {
        let bytes = decode_hex_32(hex)
            .ok_or_else(|| ActivationError::MalformedKey("chave pública hex inválida".into()))?;
        let key = VerifyingKey::from_bytes(&bytes)
            .map_err(|_| ActivationError::MalformedKey("chave pública fora da curva".into()))?;
        Ok(Self { key: Some(key) })
    }

    /// Verifica `signature` (64 bytes) sobre os bytes canônicos dos claims.
    pub fn verify_claims(&self, claims: &LicenseClaims, signature: &[u8]) -> bool {
        let Some(key) = &self.key else {
            return false;
        };
        let Ok(sig_bytes) = <[u8; 64]>::try_from(signature) else {
            return false;
        };
        let sig = Signature::from_bytes(&sig_bytes);
        key.verify_strict(&claims.canonical_bytes(), &sig).is_ok()
    }

    /// Faz parse + verificação de um token de chave. Retorna os claims e a
    /// assinatura bruta (para persistir no license.json).
    pub fn parse_token(&self, token: &str) -> Result<(LicenseClaims, Vec<u8>), ActivationError> {
        let token = token.trim();
        let mut parts = token.split('.');
        let (prefix, payload_b64, sig_b64) =
            match (parts.next(), parts.next(), parts.next(), parts.next()) {
                (Some(p), Some(payload), Some(sig), None) => (p, payload, sig),
                _ => {
                    return Err(ActivationError::MalformedKey(
                        "formato não é LINA1.<dados>.<assinatura>".into(),
                    ))
                }
            };
        if prefix != TOKEN_PREFIX {
            return Err(ActivationError::MalformedKey(format!(
                "prefixo `{prefix}` desconhecido (esperado {TOKEN_PREFIX})"
            )));
        }
        let payload = URL_SAFE_NO_PAD
            .decode(payload_b64)
            .map_err(|_| ActivationError::MalformedKey("dados da chave ilegíveis".into()))?;
        let signature = URL_SAFE_NO_PAD
            .decode(sig_b64)
            .map_err(|_| ActivationError::MalformedKey("assinatura ilegível".into()))?;
        let claims: LicenseClaims = serde_json::from_slice(&payload)
            .map_err(|_| ActivationError::MalformedKey("dados da chave não reconhecidos".into()))?;
        if claims.validate().is_err() {
            return Err(ActivationError::MalformedKey(
                "campos da chave inválidos".into(),
            ));
        }
        if !self.verify_claims(&claims, &signature) {
            return Err(ActivationError::InvalidSignature);
        }
        Ok((claims, signature))
    }
}

/// Monta a string do token a partir de claims + assinatura já feita.
/// Usado pelo `lina-keygen` na emissão (a assinatura em si acontece lá).
pub fn encode_token(claims: &LicenseClaims, signature: &[u8]) -> Result<String, serde_json::Error> {
    let payload = serde_json::to_vec(claims)?;
    Ok(format!(
        "{TOKEN_PREFIX}.{}.{}",
        URL_SAFE_NO_PAD.encode(payload),
        URL_SAFE_NO_PAD.encode(signature)
    ))
}

fn decode_hex_32(hex: &str) -> Option<[u8; 32]> {
    let hex = hex.trim();
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        let s = std::str::from_utf8(chunk).ok()?;
        out[i] = u8::from_str_radix(s, 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use rand_core::OsRng;

    fn claims() -> LicenseClaims {
        LicenseClaims {
            license_id: "id-1".into(),
            tier: "pro".into(),
            workspace_limit: None,
            entitlements: vec![],
            expiry: None,
            machine_id: None,
        }
    }

    fn signed_token(key: &SigningKey, c: &LicenseClaims) -> String {
        let sig = key.sign(&c.canonical_bytes());
        encode_token(c, &sig.to_bytes()).expect("claims serializáveis")
    }

    #[test]
    fn token_assinado_valida_e_devolve_claims() {
        let key = SigningKey::generate(&mut OsRng);
        let verifier = Verifier::with_key(key.verifying_key());
        let token = signed_token(&key, &claims());
        let (parsed, _) = verifier.parse_token(&token).expect("token válido");
        assert_eq!(parsed, claims());
    }

    #[test]
    fn keypair_de_terceiro_nao_valida() {
        let legit = SigningKey::generate(&mut OsRng);
        let attacker = SigningKey::generate(&mut OsRng);
        let verifier = Verifier::with_key(legit.verifying_key());
        let token = signed_token(&attacker, &claims());
        assert!(matches!(
            verifier.parse_token(&token),
            Err(ActivationError::InvalidSignature)
        ));
    }

    #[test]
    fn token_truncado_e_prefixo_errado_sao_malformados() {
        let key = SigningKey::generate(&mut OsRng);
        let verifier = Verifier::with_key(key.verifying_key());
        let token = signed_token(&key, &claims());
        let truncado = &token[..token.len() - 10];
        assert!(matches!(
            verifier.parse_token(truncado),
            Err(ActivationError::MalformedKey(_)) | Err(ActivationError::InvalidSignature)
        ));
        let prefixo = token.replacen("LINA1", "NOPE9", 1);
        assert!(matches!(
            verifier.parse_token(&prefixo),
            Err(ActivationError::MalformedKey(_))
        ));
    }

    #[test]
    fn official_nunca_panica_mesmo_com_const_placeholder() {
        let _ = Verifier::official();
    }
}
