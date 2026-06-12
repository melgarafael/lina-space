//! # lina-license — licença local ed25519 + gating por nº de workspaces (F1-4-5)
//!
//! Core SEM UI. Validação 100% local: chave pública embarcada no binário,
//! ZERO rede em runtime (invariante #2 local-first — diferencial deliberado,
//! ver 13.6 ação #2). A apresentação (upsell, T7) é do shell (F1-4-6).
//!
//! ## Contrato para consumidores (F1-4-1 criação de Espaço, F1-4-6 UI)
//! ```no_run
//! use lina_license::{LicenseState, WorkspaceGate};
//!
//! // Ponto de gating (boot / criação de Espaço): carregue e avalie com o
//! // relógio do chamador — expiry NUNCA é re-avaliado fora destes pontos.
//! let state = LicenseState::load_default();
//! let now = std::time::SystemTime::now()
//!     .duration_since(std::time::UNIX_EPOCH)
//!     .map(|d| d.as_secs())
//!     .unwrap_or(0);
//! match state.can_create_workspace(1, now) {
//!     WorkspaceGate::Allowed => { /* criar o Espaço */ }
//!     WorkspaceGate::Blocked { limit, tier, reason } => {
//!         let _ = (limit, tier, reason); // upsell gracioso (F1-4-6)
//!     }
//! }
//! ```
//!
//! Ativação (colar a chave): [`activate`] · estado para o painel T7:
//! [`LicenseState::effective`] · emissão de chaves: crate `lina-keygen`.

mod claims;
mod state;
mod token;

pub use claims::{ClaimsError, LicenseClaims, CLAIMS_VERSION};
pub use state::{
    activate, deactivate, default_license_path, BlockReason, EffectiveLicense, LicenseFile,
    LicenseState, LicenseStatus, WorkspaceGate, FREE_TIER, FREE_WORKSPACE_LIMIT,
};
pub use token::{encode_token, ActivationError, Verifier, OFFICIAL_PUBLIC_KEY_HEX, TOKEN_PREFIX};
