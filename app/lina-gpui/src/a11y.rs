//! `a11y` — **W4-6: acessibilidade P0 (integrativa — fecha o GATE W4)**.
//!
//! A11y **prática**, não polimento. Reusa o core (W0-2 linearização sem ANSI; W0-10 fim-de-resposta)
//! e instrumenta o shell:
//! - **AccessibleBuffer**: [`accessible_lines`] lineariza o grid do terminal em `Vec<String>` **sem
//!   ANSI** (via `VtBackend::row_text`, helper do W0-2) — a "Accessible View" legível por leitor de tela.
//! - **Live-region "resposta pronta"**: [`LiveRegion`] computa o anúncio **uma vez por turno** — pela
//!   transição de status **→Idle** (fim-de-resposta, W0-10), **NUNCA a cada byte**. ⚠️ o AUTO-anúncio
//!   ao leitor de tela não sai dos builders builtin (`div()` não expõe `set_live`) — o spike F1-2-7
//!   ([`live`], caminho (a) do ADR 0028) seta `live=Polite` via custom `Element`; validação na tela
//!   pendente (roteiro `spike-a11y-roteiro.md`) — até lá NÃO afirmar auto-anúncio como fato.
//! - **Rótulo anunciável**: [`node_label`] junta nome + o badge do W4-2 (`canvas::Badge`: "💤 dormindo",
//!   "precisa de você", "rodando") → o leitor de tela fala o estado do nó.
//! - **Contraste WCAG**: a maquinaria ([`wcag::contrast_ratio`]/[`wcag::meets_aa`]) mora aqui; o
//!   **GATE de CI** mora em `theme.rs` (F1-2-1: cobre os 2 temas, 8 acentos e todos os tokens de
//!   texto — fechou o furo "o gate guarda uma cópia" apontado pelo red-team do 13.15).
//! - **reduce-motion**: [`os_reduce_motion`] (env/SO) alimenta o `reduce_motion` do canvas (W4-3 já
//!   corta o pulso A→B quando `true`).
//!
//! Split: lógica **gpui-free e testável** + um único builder gpui ([`live_region_element`]). Invariante
//! #6 (a11y é P0, "AccessKit 1ª classe"); #2 (local-first — só lê settings locais).

use std::collections::BTreeMap;

use gpui::{prelude::*, AnyElement};
use lina_core::VtBackend;
use lina_host::{NodeId, NodeStatus};

use crate::canvas::aggregate_badge;

// F2-2-2: o live-region (ADR 0028) foi PROMOVIDO de spike a produção e do `#[path] pub mod live`
// daqui para `mod a11y_live;` no crate root (`main.rs`) — o registro agora é do dono do `main.rs`.
// Consumidores usam `crate::a11y_live::*`; o wrapper [`live_region_element`] abaixo permanece.

// ═══════════════════════════ AccessibleBuffer (W0-2: linearização sem ANSI) ═══════════════════════════

/// Lineariza o grid VISÍVEL de um terminal em `Vec<String>` **sem ANSI** (tabs→espaços já tratados
/// pelo `row_text` do `VtBackend`, helper do W0-2) — a base da "Accessible View" do leitor de tela.
/// Cada linha trimada à direita (sem padding de espaços). `allow(dead_code)`: a árvore de render do
/// gpui já expõe o texto do grid como nós acessíveis; isto é a linearização canônica, testada e pronta
/// para um controle de texto AccessKit dedicado (aprofundamento futuro — ver `.entrega-w46.md`).
#[allow(dead_code)]
#[must_use]
pub fn accessible_lines(backend: &dyn VtBackend) -> Vec<String> {
    let (_cols, rows) = backend.dims();
    (0..rows)
        .map(|r| backend.row_text(r).trim_end().to_string())
        .collect()
}

// ═══════════════════════════ rótulo acessível do nó (reusa W4-2 badge) ═══════════════════════════

/// Nome + estado **anunciável** do nó (reusa o badge do W4-2: "💤 dormindo"/"precisa de você"/"rodando").
/// É o `aria_label` do card do terminal → o leitor de tela fala identidade + estado. `needs_human`
/// (gate de custódia pendente p/ este nó, computado no render) faz "precisa de você" ser anunciável.
#[must_use]
pub fn node_label(name: &str, status: NodeStatus, needs_human: bool) -> String {
    format!(
        "{name} — {}",
        aggregate_badge(status, needs_human, 0).label()
    )
}

// ═══════════════════════════ live-region "resposta pronta" (1×/turno) ═══════════════════════════

/// Anunciador da live-region. Observa o status dos nós e, ao um nó **transitar para `Idle`**
/// (fim-de-resposta, W0-10), produz "resposta pronta" **uma vez** — não a cada byte (o status muda
/// por turno, não por `GridDelta`).
#[derive(Default)]
pub struct LiveRegion {
    last: BTreeMap<NodeId, NodeStatus>,
    current: Option<String>,
    /// Nós citados no anúncio corrente — se algum SOME, o anúncio é stale e some (não fala fantasma).
    current_nodes: Vec<NodeId>,
}

impl LiveRegion {
    /// Observa `(id, nome, status)` dos nós. Devolve o anúncio NOVO desta chamada (`Some` quando ≥1 nó
    /// transitou →Idle neste frame). **Combina TODAS as transições do frame** (não descarta as
    /// anteriores — achado do red-team) e **limpa** o anúncio se um nó citado sumiu.
    pub fn observe(&mut self, nodes: &[(NodeId, String, NodeStatus)]) -> Option<String> {
        let mut fresh: Vec<(NodeId, String)> = Vec::new();
        for (id, name, status) in nodes {
            let prev = self.last.get(id).copied();
            // Transição PARA Idle de um estado conhecido NÃO-idle = fim-de-resposta (uma vez). Um nó
            // visto Idle de primeira (prev=None) NÃO anuncia (não "acabou de terminar" do nosso ponto).
            if *status == NodeStatus::Idle && matches!(prev, Some(p) if p != NodeStatus::Idle) {
                fresh.push((*id, name.clone()));
            }
            self.last.insert(*id, *status);
        }
        // Conjunto de nós presentes neste frame.
        let present: std::collections::BTreeSet<NodeId> =
            nodes.iter().map(|(n, _, _)| *n).collect();
        self.last.retain(|id, _| present.contains(id));
        // #5: se o anúncio corrente cita um nó que SUMIU, limpa (não anuncia nó fantasma para sempre).
        if !self.current_nodes.is_empty() && self.current_nodes.iter().any(|n| !present.contains(n))
        {
            self.current = None;
            self.current_nodes.clear();
        }
        // #4: combina TODOS os nós que terminaram neste frame num único anúncio.
        if !fresh.is_empty() {
            let names: Vec<&str> = fresh.iter().map(|(_, n)| n.as_str()).collect();
            let msg = format!("{}: resposta pronta", names.join(", "));
            self.current = Some(msg.clone());
            self.current_nodes = fresh.iter().map(|(id, _)| *id).collect();
            return Some(msg);
        }
        None
    }

    /// O anúncio corrente a renderizar na live-region (persiste até o próximo / até um nó citado sumir).
    #[must_use]
    pub fn current(&self) -> Option<&str> {
        self.current.as_deref()
    }

    /// F1 (M9/toast — spec §4): anúncio **avulso** na MESMA live-region dos nós — erro de
    /// validação ao focar um campo do modal, toast de arquivamento. Não cita nó nenhum
    /// (`current_nodes` limpo), então um nó sumir do canvas não o apaga; o próximo
    /// `observe()` com transição →Idle o substitui, como qualquer anúncio.
    pub fn announce(&mut self, msg: impl Into<String>) {
        self.current = Some(msg.into());
        self.current_nodes.clear();
    }
}

// ═══════════════════════════ contraste WCAG (gate de CI ≥4.5:1) ═══════════════════════════

/// Maquinaria do **GATE WCAG** (o gate vivo é `theme::tests::themes_meet_wcag_contrast_gate`, que
/// FALHA se um tema regredir abaixo do limiar). Não é chamada em produção → `#![allow(dead_code)]`.
pub mod wcag {
    #![allow(dead_code)]

    /// Linearização sRGB de um canal (WCAG 2.x).
    fn channel_lin(c: u8) -> f64 {
        let c = c as f64 / 255.0;
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }

    /// Luminância relativa de uma cor `0xRRGGBB` (WCAG 2.x).
    fn rel_luminance(hex: u32) -> f64 {
        let r = ((hex >> 16) & 0xff) as u8;
        let g = ((hex >> 8) & 0xff) as u8;
        let b = (hex & 0xff) as u8;
        0.2126 * channel_lin(r) + 0.7152 * channel_lin(g) + 0.0722 * channel_lin(b)
    }

    /// Razão de contraste WCAG entre duas cores `0xRRGGBB` (1.0..=21.0).
    #[must_use]
    pub fn contrast_ratio(a: u32, b: u32) -> f64 {
        let (la, lb) = (rel_luminance(a), rel_luminance(b));
        let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
        (hi + 0.05) / (lo + 0.05)
    }

    /// Passa no AA para texto normal (≥ 4.5:1).
    #[must_use]
    pub fn meets_aa(fg: u32, bg: u32) -> bool {
        contrast_ratio(fg, bg) >= 4.5
    }

    /// Passa no AA para texto grande/secundário (≥ 3.0:1).
    #[must_use]
    pub fn meets_aa_large(fg: u32, bg: u32) -> bool {
        contrast_ratio(fg, bg) >= 3.0
    }
}

// ═══════════════════════════ reduce-motion (SO/env → alimenta W4-3) ═══════════════════════════

fn truthy(v: Option<&str>) -> bool {
    matches!(v, Some("1") | Some("true") | Some("yes") | Some("force"))
}

/// Lógica pura: reduce-motion ligado dado o valor de ambiente (testável sem mexer no `env`).
#[must_use]
pub fn reduce_motion_from(env_val: Option<&str>) -> bool {
    truthy(env_val)
}

/// reduce-motion do SO: hoje via `LINA_REDUCE_MOTION` (o canvas/W4-3 corta o pulso A→B quando `true`).
/// (Uma query nativa — NSWorkspace/SystemParametersInfo/gsettings — é aprofundamento; ver `.entrega`.)
#[must_use]
pub fn os_reduce_motion() -> bool {
    reduce_motion_from(std::env::var("LINA_REDUCE_MOTION").ok().as_deref())
}

/// **Fonte ÚNICA** do reduce-motion (consolidação — gap W4-6): a detecção do SO/env
/// ([`os_reduce_motion`]) é SEMPRE respeitada; o `user_override` do rodapé só pode ADICIONAR
/// reduce-motion (a11y correta: uma UI não deve REABILITAR animação que o usuário desligou no SO).
/// Canvas/pulsos (W4-3) e a11y leem DAQUI — sem duplicar a detecção.
#[must_use]
pub fn reduce_motion_effective(user_override: bool) -> bool {
    os_reduce_motion() || user_override
}

// ═══════════════════════════ elemento gpui da live-region ═══════════════════════════

/// Elemento da live-region: delega ao custom `Element` de [`crate::a11y_live`] — caminho (a) do
/// ADR 0028, SELADO na F2-0-4. O `Element` sobrescreve `Element::write_a11y_info` (hook que EXISTE
/// neste SHA, default vazio) e seta `live=Polite` + `value` (o texto que o adapter anuncia) — o que
/// os builders builtin (`div().role(Role::Status)`) NÃO faziam (`Interactivity::write_a11y_info`
/// nunca seta `live`; era o gap original do red-team). A lógica do nó é provada por teste headless;
/// o auto-anúncio NA TELA (VoiceOver falando sem foco) aguarda a validação do roteiro
/// `tasks/epico-f1/spike-a11y-roteiro.md` — NÃO afirmar "conforme ARIA live-region" até a tela
/// confirmar (gate do ADR 0028). O que já era fato segue: nó **legível ao FOCAR** e VISÍVEL (banner).
pub fn live_region_element(msg: &str) -> AnyElement {
    crate::a11y_live::live_announce(msg).into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lina_core::AlacrittyBackend;
    use uuid::Uuid;

    /// AccessibleBuffer: o grid é linearizado SEM ANSI (cores/cursor não vazam para o texto).
    #[test]
    fn accessible_lines_are_plain_text_without_ansi() {
        let mut b = AlacrittyBackend::new(20, 4);
        // texto colorido (vermelho) + reset + uma 2ª linha.
        b.advance(b"\x1b[31mhello\x1b[0m\r\nworld\r\n");
        let lines = accessible_lines(&b);
        assert_eq!(lines.len(), 4, "uma String por linha do viewport");
        assert_eq!(lines[0], "hello", "sem códigos ANSI no texto acessível");
        assert_eq!(lines[1], "world");
        assert!(
            lines[2].is_empty() && lines[3].is_empty(),
            "linhas vazias trimadas"
        );
    }

    /// Rótulo do nó anuncia o badge (reuso do W4-2): Idle → "aguardando" (BUG 1: VIVO mas idle,
    /// NUNCA "dormindo"); `needs_human` → "precisa de você".
    #[test]
    fn node_label_announces_badge_state() {
        assert!(node_label("Revisor", NodeStatus::Idle, false).contains("aguardando"));
        assert!(node_label("Dev", NodeStatus::Busy, false).contains("rodando"));
        assert!(node_label("QA", NodeStatus::Dead, false).contains("encerrado"));
        assert!(node_label("Revisor", NodeStatus::Idle, false).starts_with("Revisor —"));
        // needs_human vence o status → "precisa de você" anunciável (achado do red-team).
        assert!(node_label("Dev", NodeStatus::Idle, true).contains("precisa de você"));
    }

    /// #4 (red-team): DUAS transições →Idle no MESMO frame são AMBAS anunciadas (não se descarta uma).
    #[test]
    fn simultaneous_idle_transitions_are_all_announced() {
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        let mut lr = LiveRegion::default();
        lr.observe(&[
            (a, "A".into(), NodeStatus::Busy),
            (b, "B".into(), NodeStatus::Busy),
        ]);
        let msg = lr
            .observe(&[
                (a, "A".into(), NodeStatus::Idle),
                (b, "B".into(), NodeStatus::Idle),
            ])
            .expect("anúncio combinado");
        assert!(
            msg.contains('A') && msg.contains('B'),
            "ambos citados: {msg}"
        );
    }

    /// #5 (red-team): se o nó citado SUMIR, o anúncio é limpo (não fala nó fantasma para sempre).
    #[test]
    fn announcement_clears_when_node_vanishes() {
        let a = Uuid::now_v7();
        let mut lr = LiveRegion::default();
        lr.observe(&[(a, "A".into(), NodeStatus::Busy)]);
        lr.observe(&[(a, "A".into(), NodeStatus::Idle)]);
        assert_eq!(lr.current(), Some("A: resposta pronta"));
        // nó A sumiu (fechado) → roster vazio → o anúncio some.
        lr.observe(&[]);
        assert_eq!(lr.current(), None, "anúncio de nó sumido é limpo");
    }

    /// Live-region anuncia "resposta pronta" UMA vez por turno (transição →Idle), não por byte.
    #[test]
    fn live_region_announces_once_per_turn_not_per_byte() {
        let a = Uuid::now_v7();
        let mut lr = LiveRegion::default();
        // 1º quadro: nó Busy → nada (não terminou).
        assert_eq!(lr.observe(&[(a, "A".into(), NodeStatus::Busy)]), None);
        // ainda Busy (vários "bytes"/frames) → nada.
        assert_eq!(lr.observe(&[(a, "A".into(), NodeStatus::Busy)]), None);
        // Busy → Idle = fim-de-resposta → anuncia UMA vez.
        assert_eq!(
            lr.observe(&[(a, "A".into(), NodeStatus::Idle)]),
            Some("A: resposta pronta".to_string())
        );
        // continua Idle (frames seguintes) → NÃO re-anuncia (1×/turno).
        assert_eq!(lr.observe(&[(a, "A".into(), NodeStatus::Idle)]), None);
        assert_eq!(lr.current(), Some("A: resposta pronta"));
        // novo turno: Idle→Busy→Idle anuncia de novo.
        lr.observe(&[(a, "A".into(), NodeStatus::Busy)]);
        assert_eq!(
            lr.observe(&[(a, "A".into(), NodeStatus::Idle)]),
            Some("A: resposta pronta".to_string())
        );
    }

    /// Um nó visto Idle DE PRIMEIRA não anuncia (não "acabou de terminar" do nosso ponto de vista).
    #[test]
    fn first_seen_idle_does_not_announce() {
        let a = Uuid::now_v7();
        let mut lr = LiveRegion::default();
        assert_eq!(lr.observe(&[(a, "A".into(), NodeStatus::Idle)]), None);
    }

    /// F1 (M9): anúncio AVULSO entra na live-region e SOBREVIVE ao roster de nós (não cita nó
    /// nenhum — um nó sumir/observar de novo não o apaga). Removendo `announce()`, este falha.
    #[test]
    fn standalone_announce_reaches_live_region_and_survives_node_churn() {
        let a = Uuid::now_v7();
        let mut lr = LiveRegion::default();
        lr.observe(&[(a, "A".into(), NodeStatus::Busy)]);
        lr.announce("O diretório informado não existe.");
        assert_eq!(lr.current(), Some("O diretório informado não existe."));
        // frames seguintes sem transição não o apagam; nem o nó A sumir (anúncio não cita nós).
        lr.observe(&[(a, "A".into(), NodeStatus::Busy)]);
        lr.observe(&[]);
        assert_eq!(
            lr.current(),
            Some("O diretório informado não existe."),
            "anúncio avulso não é apagado pelo churn de nós"
        );
    }

    /// Sanidade da MAQUINARIA WCAG (extremos branco/preto e identidade). O gate de PALETA vivo é
    /// `theme::tests::themes_meet_wcag_contrast_gate` (F1-2-1: 2 temas × 8 acentos × todos os
    /// tokens de texto) — este teste só protege a matemática que aquele gate usa.
    #[test]
    fn wcag_math_sanity() {
        use wcag::contrast_ratio;
        assert!((contrast_ratio(0xffffff, 0x000000) - 21.0).abs() < 0.01);
        assert!((contrast_ratio(0x123456, 0x123456) - 1.0).abs() < 0.001);
    }

    /// reduce-motion: lógica pura por valor de ambiente.
    #[test]
    fn reduce_motion_reads_truthy_env() {
        assert!(reduce_motion_from(Some("1")));
        assert!(reduce_motion_from(Some("true")));
        assert!(!reduce_motion_from(Some("0")));
        assert!(!reduce_motion_from(None));
    }

    /// Fonte única: o override do usuário FORÇA reduce-motion (independe do SO); sem override, o efetivo
    /// é exatamente a detecção do SO (consistência — sem detecção duplicada).
    #[test]
    fn reduce_motion_effective_honors_os_and_override() {
        assert!(
            reduce_motion_effective(true),
            "override do usuário força reduce-motion qualquer que seja o SO"
        );
        assert_eq!(
            reduce_motion_effective(false),
            os_reduce_motion(),
            "sem override, o efetivo = detecção do SO (fonte única)"
        );
    }
}
