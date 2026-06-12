//! Estado da licença e gating por número de workspaces (F1-4-5).
//!
//! Regras de ouro (critérios 1/3/6/7/8 da story):
//! - Ausência de arquivo, JSON ilegível ou assinatura inválida ⇒ FREE
//!   (degradação graciosa; o app NUNCA trava por causa de licença).
//! - Expiry só é avaliado nos PONTOS DE GATING (boot / criação de Espaço) —
//!   o chamador injeta `now`, então não existe autoridade de relógio paralela
//!   rodando durante a sessão, e nada é rebaixado no meio do trabalho.
//! - Licença perpétua (`expiry: None`) é imune a qualquer relógio.
//! - O gate bloqueia CRIAR workspace além do limite; ABRIR pré-existentes
//!   nunca passa por aqui (anti-sequestro de trabalho, critério 8).

use std::path::{Path, PathBuf};

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::claims::LicenseClaims;
use crate::token::{ActivationError, Verifier};

/// Limite do tier free (decisão do Maestro: free = 1 workspace).
pub const FREE_WORKSPACE_LIMIT: u32 = 1;
/// Rótulo do tier sem licença.
pub const FREE_TIER: &str = "free";

/// Forma persistida em `~/.lina/license.json`. Os campos assinados vêm de
/// [`LicenseClaims`]; `last_validated` é metadado do app (NÃO assinado — não
/// é autoridade de nada, só telemetria local de UX).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LicenseFile {
    #[serde(flatten)]
    pub claims: LicenseClaims,
    /// Assinatura ed25519 (base64 padrão) sobre `claims.canonical_bytes()`.
    pub signature: String,
    /// Última vez (epoch s) que o app re-validou nos pontos de gating.
    #[serde(default)]
    pub last_validated: Option<u64>,
}

/// Como o estado atual foi obtido — alimenta a copy honesta de F1-4-6.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LicenseStatus {
    /// Sem `license.json`: free por padrão (não é erro).
    NoLicense,
    /// Arquivo presente, assinatura válida.
    Licensed,
    /// Arquivo presente mas ilegível (JSON quebrado, permissão).
    DegradedUnreadable,
    /// Assinatura não confere (edição manual, chave de terceiro). "Teatro"
    /// detectado — trata como free (13.6 ação #1).
    DegradedInvalidSignature,
}

/// Resultado de um ponto de gating de criação de workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceGate {
    Allowed,
    /// Bloqueio gracioso: a UI traduz em upsell honesto (F1-4-6).
    Blocked {
        limit: u32,
        tier: String,
        reason: BlockReason,
    },
}

/// Por que o gate bloqueou — cada variante é um caso de copy distinto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockReason {
    /// Sem licença válida instalada (free puro ou degradado).
    FreeTier,
    /// Licença válida, mas o limite N foi atingido.
    LimitReached,
    /// Licença autêntica porém expirada no momento do gate.
    Expired,
}

/// Visão efetiva da licença num instante `now` — o que a UI (T7) exibe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveLicense {
    pub tier: String,
    /// `None` = ilimitado.
    pub workspace_limit: Option<u32>,
    pub entitlements: Vec<String>,
    pub expiry: Option<u64>,
    /// `true` quando uma licença autêntica venceu — dispara o aviso honesto
    /// NÃO-bloqueante na primeira re-avaliação pós-expiry (critério 7).
    pub expired: bool,
    pub status: LicenseStatus,
}

/// Estado carregado da licença. Construa com [`LicenseState::load`] (ou
/// [`LicenseState::load_default`]) UMA vez por ponto de gating relevante.
#[derive(Debug, Clone)]
pub struct LicenseState {
    claims: Option<LicenseClaims>,
    status: LicenseStatus,
}

impl LicenseState {
    /// Estado free puro (sem licença).
    pub fn free() -> Self {
        Self {
            claims: None,
            status: LicenseStatus::NoLicense,
        }
    }

    /// Carrega e verifica `license.json`. NUNCA retorna erro: qualquer falha
    /// degrada para free e fica explicada em [`LicenseState::status`].
    pub fn load(path: &Path, verifier: &Verifier) -> Self {
        let raw = match std::fs::read(path) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Self::free(),
            Err(_) => {
                return Self {
                    claims: None,
                    status: LicenseStatus::DegradedUnreadable,
                }
            }
        };
        let file: LicenseFile = match serde_json::from_slice(&raw) {
            Ok(file) => file,
            Err(_) => {
                return Self {
                    claims: None,
                    status: LicenseStatus::DegradedUnreadable,
                }
            }
        };
        let Ok(signature) = B64.decode(&file.signature) else {
            return Self {
                claims: None,
                status: LicenseStatus::DegradedInvalidSignature,
            };
        };
        if file.claims.validate().is_err() || !verifier.verify_claims(&file.claims, &signature) {
            return Self {
                claims: None,
                status: LicenseStatus::DegradedInvalidSignature,
            };
        }
        Self {
            claims: Some(file.claims),
            status: LicenseStatus::Licensed,
        }
    }

    /// Carrega do caminho padrão (`~/.lina/license.json`) com a chave oficial.
    pub fn load_default() -> Self {
        match default_license_path() {
            Some(path) => Self::load(&path, &Verifier::official()),
            None => Self::free(),
        }
    }

    /// Como o estado foi obtido (para copy honesta na UI).
    pub fn status(&self) -> &LicenseStatus {
        &self.status
    }

    /// Visão efetiva no instante `now` (epoch s). Expiry é avaliado AQUI —
    /// e só aqui — preservando o critério 7 (nada rebaixa no meio da sessão;
    /// o chamador decide quando é um ponto de gating).
    pub fn effective(&self, now_epoch_s: u64) -> EffectiveLicense {
        match &self.claims {
            Some(claims) => {
                let expired = claims.expiry.is_some_and(|e| now_epoch_s > e);
                if expired {
                    EffectiveLicense {
                        tier: FREE_TIER.to_string(),
                        workspace_limit: Some(FREE_WORKSPACE_LIMIT),
                        entitlements: vec![],
                        expiry: claims.expiry,
                        expired: true,
                        status: self.status.clone(),
                    }
                } else {
                    EffectiveLicense {
                        tier: claims.tier.clone(),
                        workspace_limit: claims.workspace_limit,
                        entitlements: claims.entitlements.clone(),
                        expiry: claims.expiry,
                        expired: false,
                        status: self.status.clone(),
                    }
                }
            }
            None => EffectiveLicense {
                tier: FREE_TIER.to_string(),
                workspace_limit: Some(FREE_WORKSPACE_LIMIT),
                entitlements: vec![],
                expiry: None,
                expired: false,
                status: self.status.clone(),
            },
        }
    }

    /// PONTO DE GATING: pode criar mais um workspace, dado que já existem
    /// `existing_count`? Abrir workspaces existentes nunca consulta isto.
    pub fn can_create_workspace(&self, existing_count: u32, now_epoch_s: u64) -> WorkspaceGate {
        let eff = self.effective(now_epoch_s);
        let Some(limit) = eff.workspace_limit else {
            return WorkspaceGate::Allowed; // ilimitado
        };
        if existing_count < limit {
            return WorkspaceGate::Allowed;
        }
        let reason = if eff.expired {
            BlockReason::Expired
        } else if matches!(self.status, LicenseStatus::Licensed) {
            BlockReason::LimitReached
        } else {
            BlockReason::FreeTier
        };
        WorkspaceGate::Blocked {
            limit,
            tier: eff.tier,
            reason,
        }
    }
}

/// Ativa uma chave colada pelo usuário (F1-4-6): valida o token e persiste o
/// `license.json`. Chave vencida é recusada com erro honesto (não instala).
pub fn activate(
    key_token: &str,
    path: &Path,
    verifier: &Verifier,
    now_epoch_s: u64,
) -> Result<LicenseClaims, ActivationError> {
    let (claims, signature) = verifier.parse_token(key_token)?;
    if let Some(expiry) = claims.expiry {
        if now_epoch_s > expiry {
            return Err(ActivationError::Expired { expiry });
        }
    }
    let file = LicenseFile {
        claims: claims.clone(),
        signature: B64.encode(&signature),
        last_validated: Some(now_epoch_s),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_vec_pretty(&file).map_err(std::io::Error::other)?;
    std::fs::write(path, json)?;
    Ok(claims)
}

/// Remove a licença instalada (T7 "remover chave"). Ausência não é erro.
pub fn deactivate(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Caminho padrão `~/.lina/license.json`. A licença é estado da MÁQUINA do
/// usuário, deliberadamente fora do event log de projeto (âncora da story).
pub fn default_license_path() -> Option<PathBuf> {
    home_dir().map(|home| home.join(".lina").join("license.json"))
}

fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE").map(PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}
