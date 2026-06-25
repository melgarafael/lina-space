//! F4-1-6 · **projeção do canal WhatsApp para a tela** — "está conectado?" + a trilha de auditoria
//! narrada em pt-br ("Li 12 mensagens do grupo X" / "Enviei sua mensagem para Y").
//!
//! PURA sobre o event log (inv #4): nenhum relógio, nenhum I/O — testável com log sintético. O estado
//! de conexão vem da projeção `ChannelRegistry` do core (`ChannelStatus::Connected`); a trilha resume
//! `ChannelMessageRead`/`ChannelMessageSent` (que carregam SÓ metadados — conteúdo nunca no log). O
//! shell só DESENHA o que esta projeção devolve. Registro indecodificável é pulado (derivada, não
//! validadora — espelha `ChannelRegistry`/`Mentality`).

use lina_core::channel::{ChannelRegistry, ChannelStatus};
use lina_core::{DomainEvent, EventRecord};

/// Quantas linhas de auditoria mostrar (as mais recentes) — uma janela curta, não um histórico.
const AUDIT_MAX_LINES: usize = 6;

/// O id do canal desta onda (único canal concreto — F4-1).
const WHATSAPP: &str = "whatsapp";

/// O que a tela precisa saber do WhatsApp: se há sessão ativa + a trilha narrada (mais recente primeiro).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WhatsAppStatus {
    /// `true` enquanto há sessão conectada (badge "WhatsApp conectado" aceso).
    pub connected: bool,
    /// Linhas de auditoria humanizadas (pt-br), MAIS RECENTE primeiro, no máximo [`AUDIT_MAX_LINES`].
    pub audit: Vec<String>,
}

impl WhatsAppStatus {
    /// `true` se não há nada a mostrar (desconectado e sem trilha) — o shell omite o painel.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        !self.connected && self.audit.is_empty()
    }
}

/// Projeta o estado do WhatsApp (conexão + trilha) a partir do log. PURA.
#[must_use]
pub fn whatsapp_status(records: &[EventRecord]) -> WhatsAppStatus {
    let connected = ChannelRegistry::from_records(records)
        .get(WHATSAPP)
        .is_some_and(|c| c.status == ChannelStatus::Connected);

    let mut audit: Vec<String> = Vec::new();
    for r in records {
        let Ok(ev) = serde_json::from_value::<DomainEvent>(r.payload.clone()) else {
            continue;
        };
        match ev {
            DomainEvent::ChannelMessageRead {
                channel,
                conversation_ref,
                count,
                ..
            } if channel == WHATSAPP => audit.push(narrate_read(count, &conversation_ref)),
            DomainEvent::ChannelMessageSent {
                channel,
                conversation_ref,
                ..
            } if channel == WHATSAPP => audit.push(narrate_sent(&conversation_ref)),
            _ => {}
        }
    }
    // Mais recente primeiro, janela curta (o log cresce; a tela mostra o fim).
    audit.reverse();
    audit.truncate(AUDIT_MAX_LINES);
    WhatsAppStatus { connected, audit }
}

/// "Li N mensagens do grupo X" — plural correto; humaniza a referência da conversa (sem expor id cru
/// quando dá para dizer "grupo"/"conversa").
fn narrate_read(count: u32, conversation_ref: &str) -> String {
    let qtd = if count == 1 {
        "1 mensagem".to_string()
    } else {
        format!("{count} mensagens")
    };
    format!("Li {qtd} {}", read_phrase(conversation_ref))
}

/// "Enviei sua mensagem para X".
fn narrate_sent(conversation_ref: &str) -> String {
    format!("Enviei sua mensagem {}", sent_phrase(conversation_ref))
}

/// Frase de origem de uma leitura: `grupo:Vendas` → "do grupo Vendas"; `…@g.us` → "de um grupo";
/// senão "de uma conversa" (não vaza um id cru ao leigo).
fn read_phrase(conversation_ref: &str) -> String {
    if let Some(name) = conversation_ref.strip_prefix("grupo:") {
        format!("do grupo {name}")
    } else if conversation_ref.ends_with("@g.us") {
        "de um grupo".to_string()
    } else {
        "de uma conversa".to_string()
    }
}

/// Frase de destino de um envio: `…@g.us` → "para um grupo"; senão "para {número}" (sem o sufixo
/// técnico `@c.us`/`@lid`).
fn sent_phrase(conversation_ref: &str) -> String {
    if conversation_ref.ends_with("@g.us") {
        return "para um grupo".to_string();
    }
    let dest = conversation_ref
        .split('@')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(conversation_ref);
    format!("para {dest}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(seq: u64, ev: &DomainEvent) -> EventRecord {
        EventRecord {
            seq,
            ts: seq,
            kind: ev.kind().to_string(),
            version: ev.current_version(),
            payload: serde_json::to_value(ev).expect("serializa o evento"),
        }
    }

    fn registered() -> DomainEvent {
        DomainEvent::ChannelRegistered {
            channel: WHATSAPP.into(),
            manifest_ref: "channels/whatsapp/manifest.toml".into(),
            trust_tier: "curado".into(),
            install_ref: "noweb-2026.4.2".into(),
        }
    }
    fn connected() -> DomainEvent {
        DomainEvent::ChannelConnected {
            channel: WHATSAPP.into(),
            session_ref: "keyring:channel:whatsapp".into(),
            scope: "http://127.0.0.1:3000".into(),
        }
    }

    #[test]
    fn nothing_logged_is_empty() {
        let st = whatsapp_status(&[]);
        assert!(st.is_empty());
        assert!(!st.connected);
    }

    #[test]
    fn registered_then_connected_lights_the_badge() {
        let log = [rec(1, &registered()), rec(2, &connected())];
        let st = whatsapp_status(&log);
        assert!(st.connected, "sessão ativa → badge aceso");
    }

    #[test]
    fn disconnect_turns_the_badge_off() {
        let log = [
            rec(1, &registered()),
            rec(2, &connected()),
            rec(
                3,
                &DomainEvent::ChannelDisconnected {
                    channel: WHATSAPP.into(),
                    session_ref: "keyring:channel:whatsapp".into(),
                },
            ),
        ];
        assert!(
            !whatsapp_status(&log).connected,
            "desconectar apaga o badge"
        );
    }

    #[test]
    fn read_and_sent_become_narrated_lines_most_recent_first() {
        let log = [
            rec(1, &registered()),
            rec(2, &connected()),
            rec(
                3,
                &DomainEvent::ChannelMessageRead {
                    channel: WHATSAPP.into(),
                    conversation_ref: "grupo:Devs".into(),
                    count: 12,
                    read_by_node: "n1".into(),
                },
            ),
            rec(
                4,
                &DomainEvent::ChannelMessageSent {
                    channel: WHATSAPP.into(),
                    conversation_ref: "5511999@c.us".into(),
                    broker_exec_ref: "msg_x".into(),
                    approved_by: "@Founder".into(),
                },
            ),
        ];
        let st = whatsapp_status(&log);
        assert_eq!(st.audit.len(), 2);
        assert_eq!(
            st.audit[0], "Enviei sua mensagem para 5511999",
            "mais recente primeiro"
        );
        assert_eq!(st.audit[1], "Li 12 mensagens do grupo Devs");
    }

    #[test]
    fn singular_message_grammar_and_group_send() {
        let read1 = DomainEvent::ChannelMessageRead {
            channel: WHATSAPP.into(),
            conversation_ref: "x@g.us".into(),
            count: 1,
            read_by_node: "n1".into(),
        };
        let send_group = DomainEvent::ChannelMessageSent {
            channel: WHATSAPP.into(),
            conversation_ref: "x@g.us".into(),
            broker_exec_ref: "m".into(),
            approved_by: "@F".into(),
        };
        let st = whatsapp_status(&[rec(1, &read1), rec(2, &send_group)]);
        assert_eq!(
            st.audit[1], "Li 1 mensagem de um grupo",
            "plural correto + grupo"
        );
        assert_eq!(st.audit[0], "Enviei sua mensagem para um grupo");
    }

    #[test]
    fn audit_is_capped_to_the_window() {
        let mut log = vec![rec(1, &registered()), rec(2, &connected())];
        for i in 0..20 {
            log.push(rec(
                3 + i,
                &DomainEvent::ChannelMessageSent {
                    channel: WHATSAPP.into(),
                    conversation_ref: format!("551{i}@c.us"),
                    broker_exec_ref: "m".into(),
                    approved_by: "@F".into(),
                },
            ));
        }
        assert_eq!(
            whatsapp_status(&log).audit.len(),
            AUDIT_MAX_LINES,
            "janela curta"
        );
    }
}
