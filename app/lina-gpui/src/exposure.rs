//! F4-0-5 · **badge de exposição** — "este Espaço está falando com o mundo".
//!
//! A sinalização honesta da invariante #2 (local-first; exposição é opt-in SINALIZADO): enquanto
//! o Espaço pode alcançar o mundo externo por algum canal, a UI mostra um aviso persistente que diz
//! QUAL canal — nunca um genérico (épico 41 §III F4-0-5, doc 40 §10: cor = feedback subconsciente).
//!
//! ## Predicado de "exposto" (decisão de frente — documentada, porta aberta)
//! Um canal conta como exposto quando tem **ao menos uma credencial viva** no cofre
//! (`CredentialStored` ainda não revogado por `CredentialRevoked` do mesmo `key_ref`). Razão de ser
//! honesto: um canal só REGISTRADO (`ChannelRegistered`) mas SEM credencial não fala de fato — o
//! broker recusa o efeito externo por falta de segredo (`no_secret`, F4-0-3). É a posse do segredo
//! que torna o Espaço capaz de alcançar aquele canal; então é ela que acende o aviso. Quando a
//! semântica de "conectado" de F4-0-1/F4-0-3 (`ChannelRegistry`) amadurecer, este predicado pode
//! ser refinado (ex.: exigir registrado E credenciado E conectado) sem mudar a superfície.
//!
//! ## Projeção PURA (invariante #4 — o log é a fonte da verdade)
//! [`ExposureState::from_records`] varre `EventRecord`s crus (kind + payload JSON, padrão
//! `CostLedger`/`intelligence_adoption` — leitura direta da key, sem `from_record`) e reconstrói o
//! conjunto exposto por replay. Sem relógio, sem I/O: testável com log sintético; o `value` NUNCA
//! aparece no log (só `channel`/`key_ref`), então a projeção nunca o toca.

use std::collections::{BTreeMap, BTreeSet};

use lina_core::EventRecord;
use serde_json::Value;

/// Aviso da superfície (auditável — zero jargão; sem "keyring"/"PTY"/"scope" cru).
pub const BADGE_HEADLINE: &str = "Este Espaço está falando com o mundo";

/// Um canal pelo qual o Espaço PODE alcançar o mundo externo (tem credencial viva no cofre).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ExposedChannel {
    /// id cru do canal no log (chave de projeção; ex.: `"whatsapp"`).
    pub channel: String,
    /// nome humanizado para a superfície (ex.: `"WhatsApp"`).
    pub display: String,
}

/// O conjunto de canais expostos, derivado do event log por replay. Ordem estável (por `channel`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExposureState {
    exposed: Vec<ExposedChannel>,
}

impl ExposureState {
    /// **Coração determinístico (PURO).** Reconstrói o conjunto de canais com credencial viva por
    /// replay dos registros em ordem de log. Testável com log sintético — sem relógio, sem I/O.
    #[must_use]
    pub fn from_records(records: &[EventRecord]) -> Self {
        // Por canal, o conjunto de credenciais VIVAS (key_ref). `CredentialStored` insere;
        // `CredentialRevoked` retira; canal sem nenhuma viva deixa de existir (some do badge).
        let mut live: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for record in records {
            match record.kind.as_str() {
                "CredentialStored" => {
                    if let (Some(channel), Some(key_ref)) =
                        (str_field(record, "channel"), str_field(record, "key_ref"))
                    {
                        live.entry(channel.to_string())
                            .or_default()
                            .insert(key_ref.to_string());
                    }
                }
                "CredentialRevoked" => {
                    if let (Some(channel), Some(key_ref)) =
                        (str_field(record, "channel"), str_field(record, "key_ref"))
                    {
                        if let Some(refs) = live.get_mut(channel) {
                            refs.remove(key_ref);
                            if refs.is_empty() {
                                live.remove(channel);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        // `BTreeMap` itera as chaves em ordem estável → o badge lista canais ordenados pelo id cru.
        let exposed = live
            .into_keys()
            .map(|channel| ExposedChannel {
                display: humanize_channel(&channel),
                channel,
            })
            .collect();
        Self { exposed }
    }

    /// Nenhum canal exposto (0 → 0 badge).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.exposed.is_empty()
    }

    /// Os canais expostos, em ordem estável (o badge renderiza um chip por entrada).
    #[must_use]
    pub fn channels(&self) -> &[ExposedChannel] {
        &self.exposed
    }

    /// A frase do aviso, ou `None` quando 0 canais (sem badge). Diz QUAL(is) canal(is) — nunca um
    /// genérico (doc 40 §10): ex.: `"Este Espaço está falando com o mundo · WhatsApp"`.
    #[must_use]
    pub fn headline(&self) -> Option<String> {
        if self.exposed.is_empty() {
            return None;
        }
        let names: Vec<&str> = self.exposed.iter().map(|c| c.display.as_str()).collect();
        Some(format!("{BADGE_HEADLINE} · {}", names.join(", ")))
    }
}

/// Humaniza o id cru de um canal para a superfície (nunca jargão). Conhecidos ganham capitalização
/// de marca; o resto vira Título a partir do id (primeira letra de cada palavra em maiúscula).
#[must_use]
pub fn humanize_channel(id: &str) -> String {
    match id.trim().to_ascii_lowercase().as_str() {
        "whatsapp" => "WhatsApp".to_string(),
        "email" | "e-mail" => "E-mail".to_string(),
        "telegram" => "Telegram".to_string(),
        "slack" => "Slack".to_string(),
        "instagram" => "Instagram".to_string(),
        other => title_case(other),
    }
}

/// Título simples: cada palavra (separada por espaço/`-`/`_`) com a inicial em maiúscula.
fn title_case(s: &str) -> String {
    s.split([' ', '-', '_'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().collect::<String>() + chars.as_str()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Lê um campo string do payload cru do `EventRecord` (sem `from_record` — padrão `CostLedger`/
/// `intelligence_adoption`: leitura direta da key, robusta a versões futuras do evento).
fn str_field<'a>(record: &'a EventRecord, key: &str) -> Option<&'a str> {
    record.payload.get(key).and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rec(kind: &str, payload: serde_json::Value) -> EventRecord {
        EventRecord {
            seq: 0,
            ts: 0,
            kind: kind.to_string(),
            version: 1,
            payload,
        }
    }

    fn stored(channel: &str, key_ref: &str) -> EventRecord {
        rec(
            "CredentialStored",
            json!({"channel": channel, "scope": format!("channel:{channel}"), "key_ref": key_ref}),
        )
    }

    fn revoked(channel: &str, key_ref: &str) -> EventRecord {
        rec(
            "CredentialRevoked",
            json!({"channel": channel, "key_ref": key_ref}),
        )
    }

    #[test]
    fn zero_credentials_means_no_badge() {
        let st = ExposureState::from_records(&[]);
        assert!(st.is_empty());
        assert_eq!(st.channels().len(), 0);
        assert_eq!(st.headline(), None, "0 canais → 0 badge");
    }

    #[test]
    fn a_stored_credential_exposes_its_channel() {
        let st = ExposureState::from_records(&[stored("whatsapp", "token")]);
        assert_eq!(st.channels().len(), 1);
        assert_eq!(st.channels()[0].channel, "whatsapp");
        assert_eq!(st.channels()[0].display, "WhatsApp");
        assert_eq!(
            st.headline().as_deref(),
            Some("Este Espaço está falando com o mundo · WhatsApp")
        );
    }

    #[test]
    fn revoking_the_last_credential_drops_the_channel() {
        let st = ExposureState::from_records(&[
            stored("whatsapp", "token"),
            revoked("whatsapp", "token"),
        ]);
        assert!(st.is_empty(), "revogou a última credencial → some do badge");
    }

    #[test]
    fn a_channel_with_a_second_live_credential_stays_exposed() {
        // duas credenciais no mesmo canal; revogar uma NÃO derruba o canal.
        let st = ExposureState::from_records(&[
            stored("email", "smtp"),
            stored("email", "imap"),
            revoked("email", "smtp"),
        ]);
        assert_eq!(
            st.channels().len(),
            1,
            "ainda há um segredo vivo p/ o canal"
        );
        assert_eq!(st.channels()[0].display, "E-mail");
    }

    #[test]
    fn multiple_channels_are_listed_sorted_and_named() {
        let st =
            ExposureState::from_records(&[stored("whatsapp", "token"), stored("email", "smtp")]);
        assert_eq!(st.channels().len(), 2);
        // ordem estável por id cru: email < whatsapp.
        assert_eq!(st.channels()[0].display, "E-mail");
        assert_eq!(st.channels()[1].display, "WhatsApp");
        assert_eq!(
            st.headline().as_deref(),
            Some("Este Espaço está falando com o mundo · E-mail, WhatsApp")
        );
    }

    #[test]
    fn unrelated_event_kinds_are_ignored() {
        let st = ExposureState::from_records(&[
            rec("NodeAdded", json!({"id": "n1"})),
            stored("whatsapp", "token"),
            rec("GoalDefined", json!({"goal_id": "g1"})),
        ]);
        assert_eq!(
            st.channels().len(),
            1,
            "eventos não-credenciais não afetam a exposição"
        );
    }

    #[test]
    fn an_unknown_channel_id_is_title_cased() {
        let st = ExposureState::from_records(&[stored("minha-loja", "key")]);
        assert_eq!(st.channels()[0].display, "Minha Loja");
    }
}
