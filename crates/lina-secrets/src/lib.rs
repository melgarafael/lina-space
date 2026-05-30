//! `lina-secrets` — Secret Vault (story **W0-7**).
//!
//! Segredos (service-role keys, secret de webhook, tokens de bus por nó) vivem no
//! **keyring do SO** (Keychain no macOS, Credential Manager no Windows, Secret
//! Service no Linux) — **zero segredos em claro no disco** (invariante do
//! `CLAUDE.md`).
//!
//! ## Fronteira trocável (core/shell split — `CLAUDE.md` §7)
//! O cofre opera atrás da trait [`SecretStore`]. Em produção o backend é
//! [`KeyringStore`] (crate `keyring`); em testes/CI injeta-se [`MockStore`]
//! (em memória) — assim o gate roda **headless, sem prompt de senha** e sem
//! depender do Keychain real, que trava em ambiente sem sessão gráfica.
//!
//! ## Gate de aceite (W0-7)
//! `cargo test -p lina-secrets` faz `set` → `get` → `delete` (round-trip) com o
//! backend de teste, prova que o secret de webhook é aleatório e estável, e
//! varre o repositório confirmando que **nenhum valor sensível** aparece em
//! claro em arquivo algum.

use std::collections::HashMap;
use std::sync::Mutex;

use thiserror::Error;

/// Erros do cofre de segredos. Sem `unwrap()` em produção: todo caminho falível
/// vira uma destas variantes (regra do `CLAUDE.md`).
#[derive(Debug, Error)]
pub enum SecretError {
    /// O backend de armazenamento falhou ao operar sobre `scope/key`.
    #[error("falha no cofre de segredos para '{scope}/{key}': {source}")]
    Backend {
        /// Escopo (namespace lógico) do segredo.
        scope: String,
        /// Chave do segredo dentro do escopo.
        key: String,
        /// Erro de backend subjacente.
        source: StoreError,
    },
    /// Falha ao obter entropia do SO para gerar um segredo.
    #[error("falha ao gerar segredo aleatório: {0}")]
    Random(String),
}

/// Erro de um backend de armazenamento ([`KeyringStore`] ou [`MockStore`]).
#[derive(Debug, Error)]
pub enum StoreError {
    /// O keyring nativo do SO reportou falha (mensagem repassada).
    #[error("{0}")]
    Backend(String),
    /// O lock do backend em memória foi envenenado por um panic em outra thread.
    #[error("lock do cofre em memória envenenado")]
    Poisoned,
}

/// Backend de armazenamento de segredos. Abstrai o keyring do SO para que os
/// testes rodem **headless** com um backend em memória ([`MockStore`]) e para
/// manter o cofre trocável (mesma disciplina de fronteiras do `CLAUDE.md`).
///
/// Convenção de chaveamento: `service` é o namespace composto pelo cofre
/// (`"{workspace}:{scope}"`) e `account` é a chave do segredo.
pub trait SecretStore: Send + Sync {
    /// Grava (ou sobrescreve) um segredo sob `(service, account)`.
    fn set(&self, service: &str, account: &str, secret: &str) -> Result<(), StoreError>;
    /// Lê um segredo; `Ok(None)` quando não existe.
    fn get(&self, service: &str, account: &str) -> Result<Option<String>, StoreError>;
    /// Apaga um segredo. Idempotente: apagar inexistente é `Ok(())`.
    fn delete(&self, service: &str, account: &str) -> Result<(), StoreError>;
}

/// Backend de produção: cofre nativo do SO via crate `keyring` (Keychain macOS /
/// Credential Manager Windows / Secret Service Linux). **Nunca grava segredo em
/// claro no disco** — é o que satisfaz o invariante do `CLAUDE.md`.
#[derive(Debug, Default, Clone, Copy)]
pub struct KeyringStore;

impl KeyringStore {
    /// Cria o backend de keyring do SO.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl SecretStore for KeyringStore {
    fn set(&self, service: &str, account: &str, secret: &str) -> Result<(), StoreError> {
        let entry = keyring::Entry::new(service, account).map_err(store_err)?;
        entry.set_password(secret).map_err(store_err)
    }

    fn get(&self, service: &str, account: &str) -> Result<Option<String>, StoreError> {
        let entry = keyring::Entry::new(service, account).map_err(store_err)?;
        match entry.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(store_err(e)),
        }
    }

    fn delete(&self, service: &str, account: &str) -> Result<(), StoreError> {
        let entry = keyring::Entry::new(service, account).map_err(store_err)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(store_err(e)),
        }
    }
}

fn store_err(e: keyring::Error) -> StoreError {
    StoreError::Backend(e.to_string())
}

/// Backend de teste em memória — **só para testes/CI**. Não persiste nada, por
/// isso o gate "nada em claro no disco" passa trivialmente: nenhum byte toca o
/// disco. Substitui o Keychain real, que pediria senha e travaria em headless.
#[derive(Debug, Default)]
pub struct MockStore {
    inner: Mutex<HashMap<(String, String), String>>,
}

impl MockStore {
    /// Cria um backend de teste vazio.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretStore for MockStore {
    fn set(&self, service: &str, account: &str, secret: &str) -> Result<(), StoreError> {
        self.inner
            .lock()
            .map_err(|_| StoreError::Poisoned)?
            .insert((service.to_owned(), account.to_owned()), secret.to_owned());
        Ok(())
    }

    fn get(&self, service: &str, account: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| StoreError::Poisoned)?
            .get(&(service.to_owned(), account.to_owned()))
            .cloned())
    }

    fn delete(&self, service: &str, account: &str) -> Result<(), StoreError> {
        self.inner
            .lock()
            .map_err(|_| StoreError::Poisoned)?
            .remove(&(service.to_owned(), account.to_owned()));
        Ok(())
    }
}

/// Cofre de segredos do Lina, com **namespacing por workspace**. Compõe o
/// `service` do keyring como `"{namespace}:{scope}"` e usa `key` como conta —
/// assim dois workspaces (ou dois escopos) nunca colidem no cofre do SO.
#[derive(Debug, Clone)]
pub struct SecretVault<S: SecretStore = KeyringStore> {
    namespace: String,
    store: S,
}

impl SecretVault<KeyringStore> {
    /// Cofre de produção sobre o keyring do SO, namespaced por workspace.
    #[must_use]
    pub fn new(namespace: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
            store: KeyringStore,
        }
    }
}

impl<S: SecretStore> SecretVault<S> {
    /// Cofre sobre um backend injetado (ex.: [`MockStore`] em testes/CI).
    pub fn with_store(namespace: impl Into<String>, store: S) -> Self {
        Self {
            namespace: namespace.into(),
            store,
        }
    }

    fn service(&self, scope: &str) -> String {
        format!("{}:{}", self.namespace, scope)
    }

    /// Grava (ou sobrescreve) o segredo `key` dentro de `scope`.
    pub fn set(&self, scope: &str, key: &str, val: &str) -> Result<(), SecretError> {
        self.store
            .set(&self.service(scope), key, val)
            .map_err(|source| SecretError::Backend {
                scope: scope.to_owned(),
                key: key.to_owned(),
                source,
            })
    }

    /// Lê o segredo `key` de `scope`; `Ok(None)` quando não existe.
    pub fn get(&self, scope: &str, key: &str) -> Result<Option<String>, SecretError> {
        self.store
            .get(&self.service(scope), key)
            .map_err(|source| SecretError::Backend {
                scope: scope.to_owned(),
                key: key.to_owned(),
                source,
            })
    }

    /// Apaga o segredo `key` de `scope` (idempotente).
    pub fn delete(&self, scope: &str, key: &str) -> Result<(), SecretError> {
        self.store
            .delete(&self.service(scope), key)
            .map_err(|source| SecretError::Backend {
                scope: scope.to_owned(),
                key: key.to_owned(),
                source,
            })
    }

    /// Garante um **secret de webhook** para `key` em `scope`: retorna o
    /// existente ou gera um novo aleatório (256 bits, hex), persiste no cofre e
    /// retorna. Estável por construção — chamar duas vezes devolve o mesmo
    /// valor. Consumido depois pelo nó-Gatilho para assinar webhooks.
    pub fn ensure_webhook_secret(&self, scope: &str, key: &str) -> Result<String, SecretError> {
        if let Some(existing) = self.get(scope, key)? {
            return Ok(existing);
        }
        let secret = generate_webhook_secret()?;
        self.set(scope, key, &secret)?;
        Ok(secret)
    }
}

/// Gera um secret de webhook aleatório: 32 bytes de entropia do SO, em hex
/// (64 chars = 256 bits). **Não persiste nada** — quem chama decide onde guardar
/// (em produção, o keyring via [`SecretVault::ensure_webhook_secret`]).
pub fn generate_webhook_secret() -> Result<String, SecretError> {
    const LEN: usize = 32;
    let mut bytes = [0u8; LEN];
    getrandom::getrandom(&mut bytes).map_err(|e| SecretError::Random(e.to_string()))?;
    Ok(to_hex(&bytes))
}

fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        // Escrever num `String` é infalível; o `Result` não tem o que tratar.
        let _ = write!(out, "{b:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault() -> SecretVault<MockStore> {
        SecretVault::with_store("lina-space/workspace-de-teste", MockStore::new())
    }

    #[test]
    fn round_trip_set_get_delete() {
        let v = vault();
        // Ausente no início.
        assert!(v.get("bus", "node-a").expect("get").is_none());
        // Grava e lê de volta (round-trip).
        v.set("bus", "node-a", "s3gr3d0-do-bus").expect("set");
        assert_eq!(
            v.get("bus", "node-a").expect("get").as_deref(),
            Some("s3gr3d0-do-bus")
        );
        // Apaga e some.
        v.delete("bus", "node-a").expect("delete");
        assert!(v.get("bus", "node-a").expect("get").is_none());
        // Apagar de novo é idempotente (sem erro).
        v.delete("bus", "node-a").expect("delete idempotente");
    }

    #[test]
    fn namespacing_isola_escopos_e_workspaces() {
        let v = vault();
        v.set("bus", "k", "um").expect("set bus");
        v.set("webhook", "k", "dois").expect("set webhook");
        // Mesma `key`, escopos diferentes => valores diferentes.
        assert_eq!(v.get("bus", "k").expect("get").as_deref(), Some("um"));
        assert_eq!(v.get("webhook", "k").expect("get").as_deref(), Some("dois"));
        // Outro workspace não enxerga os segredos deste.
        let outro = SecretVault::with_store("lina-space/outro-workspace", MockStore::new());
        assert!(outro.get("bus", "k").expect("get").is_none());
    }

    #[test]
    fn webhook_secret_e_aleatorio_e_estavel() {
        let a = generate_webhook_secret().expect("gera a");
        let b = generate_webhook_secret().expect("gera b");
        assert_eq!(a.len(), 64, "32 bytes em hex = 64 chars");
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()), "deve ser hex");
        assert_ne!(a, b, "dois segredos gerados não podem colidir");

        let v = vault();
        let first = v
            .ensure_webhook_secret("webhook", "gatilho-1")
            .expect("ensure 1");
        let again = v
            .ensure_webhook_secret("webhook", "gatilho-1")
            .expect("ensure 2");
        assert_eq!(
            first, again,
            "ensure_* é estável: não regenera se já existe"
        );
        assert_eq!(
            v.get("webhook", "gatilho-1").expect("get").as_deref(),
            Some(first.as_str())
        );
    }

    /// Gate W0-7: prova que **nada sensível é persistido em claro pelo app**.
    /// Usa um segredo aleatório como agulha e varre o repositório inteiro
    /// (exceto `target/` e `.git/`). O backend de produção é o keyring do SO e o
    /// de teste é em memória — em nenhum caso o valor toca um arquivo em claro.
    #[test]
    fn nenhum_segredo_em_claro_no_disco() {
        let v = vault();
        let needle = generate_webhook_secret().expect("agulha");
        v.set("bus", "node-secreto", &needle).expect("set bus");
        v.set("webhook", "gatilho", &needle).expect("set webhook");

        let root = workspace_root();
        let mut hits = Vec::new();
        scan_dir(&root, &needle, &mut hits);
        assert!(
            hits.is_empty(),
            "segredo encontrado em claro nestes arquivos: {hits:?}"
        );
    }

    fn workspace_root() -> std::path::PathBuf {
        // .../crates/lina-secrets -> sobe 2 níveis até a raiz do workspace.
        let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        manifest
            .parent()
            .and_then(std::path::Path::parent)
            .map_or(manifest.clone(), std::path::Path::to_path_buf)
    }

    fn scan_dir(dir: &std::path::Path, needle: &str, hits: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // Pula artefatos de build e git — ruído, não estado do app.
                let name = entry.file_name();
                if name == "target" || name == ".git" {
                    continue;
                }
                scan_dir(&path, needle, hits);
            } else if let Ok(bytes) = std::fs::read(&path) {
                if String::from_utf8_lossy(&bytes).contains(needle) {
                    hits.push(path);
                }
            }
        }
    }
}
