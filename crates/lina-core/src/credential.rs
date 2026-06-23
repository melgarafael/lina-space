//! F4-0-2 (ADR 0004 custódia · spec 53 §credencial) — **binding cofre↔log das credenciais de canal.**
//!
//! Une as duas pontas que nenhuma sozinha pode unir: o cofre (`lina-secrets`, que guarda o VALOR no
//! keyring) e o event log (que registra o FATO com a REFERÊNCIA). [`store_channel_credential`] guarda
//! a credencial E apenda `CredentialStored{channel,scope,key_ref}`; [`revoke_channel_credential`]
//! revoga E apenda `CredentialRevoked{channel,key_ref}`. **O VALOR nunca entra no log** — só a
//! `key_ref` da [`lina_secrets::CredentialRef`] (invariante #2 "nada em claro no disco").
//!
//! É o ponto ÚNICO que o app (upload pela UI — F4-0-2-UI) e, na revogação, o broker chamam; o agente
//! jamais. Que o valor não vaza para o env de um agente está provado por inspeção do processo-filho
//! real em `tests/f4_0_cred_env_dump.rs` (gate (a), critério inforjável). O backup `.bak` da credencial
//! anterior é fato do cofre (`lina-secrets`), não evento.

use lina_secrets::{SecretError, SecretStore, SecretVault};
use thiserror::Error;

use crate::events::{DomainEvent, EventStore, StoreError};

/// Falha do binding: ou o cofre, ou o append do evento. Mantém as duas causas distinguíveis (o caller
/// sabe se o segredo foi guardado mas o log falhou, p.ex.).
#[derive(Debug, Error)]
pub enum CredentialBindingError {
    /// O cofre de segredos (`lina-secrets`) falhou.
    #[error("cofre de segredos: {0}")]
    Vault(#[from] SecretError),
    /// O append no event log falhou.
    #[error("event log: {0}")]
    Store(#[from] StoreError),
}

/// **Guarda a credencial `key` do `channel` no cofre E registra o fato no log.** O valor vai para o
/// keyring (`lina-secrets`, namespaced por workspace); o log recebe só `CredentialStored{channel,
/// scope, key_ref}` — a REFERÊNCIA, JAMAIS o valor (ADR 0004 / invariante #2). Backup `.bak` da
/// anterior é silenciosamente preservado pelo cofre antes de sobrescrever (Hermes doc 40 §3).
///
/// # Errors
/// [`CredentialBindingError::Vault`] se o cofre falhar; [`CredentialBindingError::Store`] se o append
/// falhar (o segredo já estará no cofre — o caller decide se compensa).
pub fn store_channel_credential<S: SecretStore>(
    vault: &SecretVault<S>,
    store: &mut EventStore,
    channel: &str,
    key: &str,
    val: &str,
) -> Result<(), CredentialBindingError> {
    let cred = vault.set_channel_credential(channel, key, val)?;
    store.append(&DomainEvent::CredentialStored {
        channel: cred.channel,
        scope: cred.scope,
        key_ref: cred.key_ref,
    })?;
    Ok(())
}

/// **Revoga a credencial `key` do `channel` no cofre E registra `CredentialRevoked{channel,key_ref}`.**
/// O cofre apaga a entrada E o seu `.bak` (revogar não deixa segredo por caminho esquecido).
/// Idempotente do lado do cofre; o evento sempre registra o pedido de revogação.
///
/// # Errors
/// [`CredentialBindingError::Vault`] se o cofre falhar; [`CredentialBindingError::Store`] se o append falhar.
pub fn revoke_channel_credential<S: SecretStore>(
    vault: &SecretVault<S>,
    store: &mut EventStore,
    channel: &str,
    key: &str,
) -> Result<(), CredentialBindingError> {
    let cred = vault.revoke_channel_credential(channel, key)?;
    store.append(&DomainEvent::CredentialRevoked {
        channel: cred.channel,
        key_ref: cred.key_ref,
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lina_secrets::MockStore;

    fn store(tag: &str) -> EventStore {
        let dir = std::env::temp_dir().join(format!(
            "lina-cred-bind-{}-{}-{:?}",
            tag,
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        EventStore::open(&dir).expect("abre o event store de teste")
    }

    fn vault() -> SecretVault<MockStore> {
        SecretVault::with_store("lina-space/ws-f40", MockStore::new())
    }

    #[test]
    fn store_puts_value_in_vault_and_only_reference_in_log() {
        let v = vault();
        let mut s = store("store");
        store_channel_credential(&v, &mut s, "whatsapp", "token", "s3gr3do-real")
            .expect("guarda + registra");

        // O VALOR foi pro cofre (o broker o lê na execução; o agente não).
        assert_eq!(
            v.get_channel_credential("whatsapp", "token")
                .expect("get")
                .as_deref(),
            Some("s3gr3do-real")
        );
        // O log tem CredentialStored com a REFERÊNCIA — nunca o valor.
        let recs = s.events().expect("events");
        let rec = recs
            .iter()
            .find(|r| r.kind == "CredentialStored")
            .expect("CredentialStored no log");
        let payload = serde_json::to_string(&rec.payload).expect("serializa");
        assert!(
            !payload.contains("s3gr3do-real"),
            "valor vazou no log: {payload}"
        );
        assert!(payload.contains("\"channel:whatsapp\""), "escopo no log");
        assert!(payload.contains("\"token\""), "key_ref no log");
    }

    #[test]
    fn revoke_removes_secret_and_logs_reference() {
        let v = vault();
        let mut s = store("revoke");
        store_channel_credential(&v, &mut s, "email", "api_key", "v1").expect("guarda");
        revoke_channel_credential(&v, &mut s, "email", "api_key").expect("revoga + registra");

        // Sumiu do cofre.
        assert!(v
            .get_channel_credential("email", "api_key")
            .expect("get")
            .is_none());
        // CredentialRevoked (estruturalmente sem campo de valor) no log.
        let recs = s.events().expect("events");
        assert!(
            recs.iter().any(|r| r.kind == "CredentialRevoked"),
            "CredentialRevoked apendado"
        );
    }
}
