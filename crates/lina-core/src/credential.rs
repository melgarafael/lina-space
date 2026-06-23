//! F4-0-2 · frente CREDENCIAIS (dono: Terminal I) — **binding cofre↔event-log**.
//!
//! STUB DA LARGADA F4-0 (preencher nesta frente). A PONTE entre o cofre (`lina-secrets`, onde a
//! credencial-de-canal vive no keyring do SO) e o event-log (`lina-core`, onde só a REFERÊNCIA é
//! registrada). O `broker.rs` já depende de `lina_secrets` — este módulo segue o mesmo precedente.
//!
//! ## O que esta frente entrega (critérios F4-0-2, épico 41 §III · ADR 0004)
//! - **`store_channel_credential(vault, store, channel, scope, key, value)`** — grava no `SecretVault`
//!   (scope `channel:<nome>`), faz **backup `.bak`** antes de sobrescrever (Hermes doc 40 §3), e
//!   emite `DomainEvent::CredentialStored { channel, scope, key_ref }` no `EventStore`. Retorna um
//!   `CredentialRef { channel, scope, key_ref }` — a referência, NUNCA o valor.
//! - **`revoke_channel_credential(vault, store, channel, scope, key)`** — apaga do cofre e emite
//!   `CredentialRevoked { channel, key_ref }`.
//!
//! ## Invariantes inegociáveis (ADR 0004 · risco #1 da fase — o red-team re-deriva)
//! - **O VALOR da credencial NUNCA cruza esta fronteira para o log.** Só `key_ref` (a chave/conta no
//!   keyring) entra no `CredentialStored` — `assert` no teste de que o payload não contém o valor.
//! - **O valor NUNCA vai para o env de PTY de agente.** A custódia injeta o segredo só no momento da
//!   execução brokerada (F4-0-3); o agente nunca o vê (gate (a): dump de env = 0 hits).
//! - **`store.append` é o ÚNICO ponto de emissão** — o `key_ref` é derivado de `(channel, scope, key)`,
//!   não escrito por agente; nenhum campo do payload decide identidade/autorização.
//!
//! Eventos: `DomainEvent::CredentialStored`/`CredentialRevoked` (congelados na largada — não
//! re-editar `events.rs`). Helper de cofre puro (set/get/revoke sobre `channel:<nome>`) fica em
//! `lina-secrets` (território de I, sem costura).
