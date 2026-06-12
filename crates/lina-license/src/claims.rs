//! Claims da licença e sua forma canônica assinável.
//!
//! A assinatura ed25519 cobre `canonical_bytes(claims)` — uma serialização
//! determinística própria, NÃO o JSON do arquivo. JSON não tem forma canônica
//! garantida (ordem de chaves, whitespace); assinar bytes derivados dos campos
//! garante que editar qualquer campo no `license.json` sem re-assinar invalida
//! a licença (ação #1 do 13.6: "sem assinatura, o feature-gating é teatro").

use serde::{Deserialize, Serialize};

/// Versão do esquema de assinatura. Entra nos bytes canônicos: uma licença v1
/// nunca valida contra um verificador que espere outra forma canônica.
pub const CLAIMS_VERSION: &str = "lina-license-v1";

/// Os campos ASSINADOS da licença (data-driven, ação #3 do 13.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LicenseClaims {
    /// Identificador único da chave (uma por aluno/compra; sem master key).
    pub license_id: String,
    /// Rótulo do plano ("pro", "free", futuros). Dado, não autoridade de UI.
    pub tier: String,
    /// Gate primário: nº máximo de workspaces. `None` = ilimitado.
    #[serde(default)]
    pub workspace_limit: Option<u32>,
    /// Entitlements adicionais data-driven (features futuras sem re-release).
    #[serde(default)]
    pub entitlements: Vec<String>,
    /// Expiração em epoch-seconds UTC. `None` = perpétua (imune a relógio).
    #[serde(default)]
    pub expiry: Option<u64>,
    /// Porta para node-locking futuro (ação #9). NÃO validado na Fase 1.
    #[serde(default)]
    pub machine_id: Option<String>,
}

/// Erros de forma dos claims (campos que quebrariam a forma canônica).
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ClaimsError {
    #[error("campo `{0}` vazio")]
    EmptyField(&'static str),
    #[error("campo `{field}` contém caractere proibido na forma canônica: {value:?}")]
    ForbiddenChars { field: &'static str, value: String },
}

/// Caracteres que quebrariam o encoding canônico (delimitadores de linha/lista).
fn has_forbidden_chars(value: &str) -> bool {
    value.chars().any(|c| c == '\n' || c == '\r' || c == ',')
}

impl LicenseClaims {
    /// Valida a FORMA dos campos (não a assinatura). Chamado na emissão
    /// (keygen) e no parse: claims malformados nunca chegam à verificação.
    pub fn validate(&self) -> Result<(), ClaimsError> {
        for (field, value) in [("license_id", &self.license_id), ("tier", &self.tier)] {
            if value.is_empty() {
                return Err(ClaimsError::EmptyField(field));
            }
        }
        let optional = [
            ("license_id", Some(&self.license_id)),
            ("tier", Some(&self.tier)),
            ("machine_id", self.machine_id.as_ref()),
        ];
        for (field, value) in optional.into_iter() {
            if let Some(value) = value {
                if has_forbidden_chars(value) {
                    return Err(ClaimsError::ForbiddenChars {
                        field,
                        value: value.clone(),
                    });
                }
            }
        }
        for ent in &self.entitlements {
            if ent.is_empty() {
                return Err(ClaimsError::EmptyField("entitlements"));
            }
            if has_forbidden_chars(ent) {
                return Err(ClaimsError::ForbiddenChars {
                    field: "entitlements",
                    value: ent.clone(),
                });
            }
        }
        Ok(())
    }

    /// Bytes determinísticos sobre os quais a assinatura é feita/verificada.
    /// Um campo por linha, ordem fixa, `Option` vazio vira string vazia.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let limit = self
            .workspace_limit
            .map(|n| n.to_string())
            .unwrap_or_default();
        let expiry = self.expiry.map(|e| e.to_string()).unwrap_or_default();
        let machine_id = self.machine_id.clone().unwrap_or_default();
        format!(
            "{CLAIMS_VERSION}\nlicense_id={}\ntier={}\nworkspace_limit={limit}\nentitlements={}\nexpiry={expiry}\nmachine_id={machine_id}\n",
            self.license_id,
            self.tier,
            self.entitlements.join(","),
        )
        .into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claims() -> LicenseClaims {
        LicenseClaims {
            license_id: "id-1".into(),
            tier: "pro".into(),
            workspace_limit: Some(3),
            entitlements: vec!["beta".into()],
            expiry: Some(100),
            machine_id: None,
        }
    }

    #[test]
    fn canonical_muda_quando_qualquer_campo_assinado_muda() {
        let base = claims().canonical_bytes();
        let mut c = claims();
        c.tier = "free".into();
        assert_ne!(base, c.canonical_bytes());
        let mut c = claims();
        c.workspace_limit = Some(99);
        assert_ne!(base, c.canonical_bytes());
        let mut c = claims();
        c.expiry = None;
        assert_ne!(base, c.canonical_bytes());
        let mut c = claims();
        c.entitlements = vec![];
        assert_ne!(base, c.canonical_bytes());
    }

    #[test]
    fn valida_recusa_delimitadores_que_quebrariam_o_canonico() {
        let mut c = claims();
        c.tier = "pro,extra".into();
        assert!(matches!(
            c.validate(),
            Err(ClaimsError::ForbiddenChars { .. })
        ));
        let mut c = claims();
        c.entitlements = vec!["a\nb".into()];
        assert!(matches!(
            c.validate(),
            Err(ClaimsError::ForbiddenChars { .. })
        ));
        assert_eq!(claims().validate(), Ok(()));
    }
}
