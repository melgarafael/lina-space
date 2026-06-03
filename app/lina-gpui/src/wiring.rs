//! **W4-3 — chrome de conexão "sem fios" (gpui-free).** O diferencial #1 visível: estar no Espaço =
//! estar conectado, SEM ligar cabos. Modela (puro e testável):
//! - o **FREIO** (pausa da auto-orquestração) — estado compartilhado entre a UI (que pede) e a
//!   `MailboxPump` (que aplica no `Router` e espelha), no mesmo padrão da mesa de custódia (round 5);
//! - o **SELO "Time conectado"** — full-mesh LÓGICO derivado da MEMBERSHIP (decisão arq §2.2: sem
//!   arestas persistentes; a conexão vem da presença, não de cabos);
//! - a gate de **reduce-motion** do pulso (a11y, W4-6 liga ao SO; aqui fica o hook).
//!
//! O render gpui e a `MailboxPump` CONSOMEM este modelo. O pulso efêmero A→B em si é decorativo
//! (a entrega acontece independente dele).

use std::sync::{Arc, Mutex};

/// **Estado do FREIO**, compartilhado entre a UI e a [`crate::bridge::MailboxPump`]. A UI NUNCA toca o
/// `Router`: só sinaliza `toggle_requested`; a pump aplica `Router::pause`/`resume` e espelha `paused`
/// (que a UI lê para o rótulo). Pausado = delegações novas ENFILEIRAM (não injetam) até retomar.
#[derive(Debug, Default)]
pub struct BrakeState {
    /// Espelho read-only (p/ a UI) de `Router::is_paused` — atualizado pela pump.
    pub paused: bool,
    /// A UI pediu para ALTERNAR o freio (pausa↔retoma). A pump consome (e zera) no próximo tick.
    pub toggle_requested: bool,
}

/// Handle compartilhado do freio.
pub type Brake = Arc<Mutex<BrakeState>>;

/// Cria um freio novo (despausado). A pump pode sobrescrever `paused` ao restaurar o estado do log.
#[must_use]
pub fn new_brake() -> Brake {
    Arc::new(Mutex::new(BrakeState::default()))
}

/// **Selo "Time conectado" (full-mesh LÓGICO por membership).** 2+ nós vivos no Espaço ⇒ o time se
/// fala (a conexão é derivada da presença — sem tabela de arestas). 0-1 nó ⇒ ainda não há "time".
#[must_use]
pub fn team_connected(live_node_count: usize) -> bool {
    live_node_count >= 2
}

/// `true` se o pulso A→B deve ANIMAR. Com **reduce-motion** ligado (a11y), a animação é suprimida —
/// a entrega ainda ocorre (o pulso é decorativo), apenas não há movimento na tela.
#[must_use]
pub fn animate_pulse(reduce_motion: bool) -> bool {
    !reduce_motion
}

#[cfg(test)]
mod tests {
    use super::*;

    /// O selo é full-mesh por membership: aparece com 2+ nós, some com 0-1.
    #[test]
    fn team_connected_is_membership_derived() {
        assert!(!team_connected(0), "Espaço vazio: sem time");
        assert!(!team_connected(1), "1 nó: ainda não há time");
        assert!(team_connected(2), "2 nós: time conectado");
        assert!(team_connected(7), "muitos nós: conectado");
    }

    /// reduce-motion suprime a animação do pulso (mas não a entrega).
    #[test]
    fn reduce_motion_suppresses_pulse_animation() {
        assert!(animate_pulse(false), "sem reduce-motion: pulso anima");
        assert!(!animate_pulse(true), "reduce-motion: pulso NÃO anima");
    }

    /// O freio nasce despausado; alternar é um pedido que a pump consome.
    #[test]
    fn brake_starts_unpaused_and_toggle_is_a_request() {
        let brake = new_brake();
        {
            let b = brake.lock().unwrap();
            assert!(!b.paused);
            assert!(!b.toggle_requested);
        }
        // A UI pede a alternância.
        brake.lock().unwrap().toggle_requested = true;
        // A pump consome (zera) e espelha o novo estado.
        let mut b = brake.lock().unwrap();
        assert!(
            std::mem::take(&mut b.toggle_requested),
            "o pedido é consumível"
        );
        assert!(!b.toggle_requested, "consumido");
    }
}
