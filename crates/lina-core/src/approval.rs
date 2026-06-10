//! F1-1-8 · Confirmação de aprovação (ADR 0021, ACEITO): snapshot da pergunta,
//! porta única de entrega validada contra a tela e idempotência por projeção do log.
//!
//! ## Mecanismo (ADR 0021 §1/§2/§5)
//! - **Captura 1 (detecção/aviso):** o chamador captura [`prompt_snapshot_hash`] quando o
//!   pedido é detectado/exibido — o hash viaja com o pedido até o gesto humano.
//! - **Captura 2 (porta única):** [`PtyHost::deliver_approval`](crate::PtyHost::deliver_approval)
//!   re-snapshota, compara e escreve **sob o mesmo lock do `VtBackend` que serializa o
//!   `advance` do flush** — nenhum byte do PTY é aplicado ao grid entre o check e o write
//!   (a atomicidade local do "mesmo turno do loop"). Tela divergente ⇒ **zero bytes**.
//! - **Idempotência:** o [`ApprovalLedger`] é projeção PURA dos eventos
//!   `PermissionAsked/Resolved/Dismissed` + `ApprovalInjected/Aborted/DuplicateIgnored` —
//!   derivado do log, jamais tabela-autoridade paralela (padrão ADR 0014/0020).
//!
//! ## Doutrina (gate humano — CLAUDE.md; ADR 0021 §5)
//! - O write é exclusivamente `approval_keys` do CLI Profile (tecla crua, sem
//!   bracketed-paste); **nenhum byte de agente entra no write** — por construção não há
//!   o que sanitizar.
//! - O binding `stable_id → node_id` vem do `PermissionAsked` no LOG (fonte interna);
//!   divergência com o PTY alvo ⇒ `ApprovalAborted{target_mismatch}` (R4).
//! - **Reinício NUNCA re-digita** (trava por construção): [`ApprovalLedger::observe`] e
//!   [`ApprovalExecutor::observe`] não têm acesso a porta de escrita — replay/boot só
//!   reconstrói estado. Digitar exige gesto fresco ([`ApprovalGesture`]) + tela conferida
//!   AGORA pela porta. Crash entre `PermissionResolved` e o write ⇒ o `stable_id` fica
//!   `resolved` no ledger e qualquer gesto retardatário é no-op auditado (§2).
//!
//! ## Contrato de K
//! O `K` (linhas da região do prompt) entra no material do hash: Captura 1 e Captura 2
//! DEVEM usar o mesmo valor (default [`PROMPT_REGION_ROWS`]) — K divergente nunca casa
//! e aborta (direção fail-safe).

use std::collections::HashMap;
use std::fmt::Write as _;

use lina_cli_profiles::ApprovalKeys;
use lina_vt::VtBackend;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::events::{ApprovalDecision, DomainEvent, ResolutionVia};

/// ADR 0021 §1: `K` default — últimas 8 linhas NÃO-vazias do viewport compõem a região
/// do prompt. Tunável (hipótese calibrável no red-team; nunca afrouxar sem red-team).
pub const PROMPT_REGION_ROWS: usize = 8;

/// Razão canônica do abort por tela divergente (ADR 0021 §1).
pub const REASON_SCREEN_CHANGED: &str = "screen_changed";
/// Razão canônica do abort por binding alvo divergente (ADR 0021 §4 R4).
pub const REASON_TARGET_MISMATCH: &str = "target_mismatch";
/// Razão do abort por erro de I/O na porta (efeito NÃO confirmado ⇒ sem `Injected`).
pub const REASON_PORT_ERROR: &str = "port_error";

// ───────────────────────── §1 · Snapshot da região do prompt ─────────────────────────

/// Hash SHA-256 da **região do prompt** (ADR 0021 §1), lido do grid PARSEADO via
/// [`VtBackend`] (células pós-emulador — escapes já consumidos), nunca de bytes crus.
///
/// Material do hash, em forma canônica:
/// - texto (trim à direita) das últimas `k` linhas **não-vazias** do grid vivo, na ordem
///   do viewport (linhas em branco são puladas, não contam para `k`);
/// - dimensões `(cols, rows)`;
/// - posição do cursor **no grid vivo** (linha/coluna);
/// - o próprio `k` (capturas com `K` divergente nunca casam — fail-safe).
///
/// **Fora do hash, por decisão:** atributos de cor/estilo (re-render e troca de tema não
/// mudam a semântica do prompt — ADR §1) e o `display_offset` de scroll (rolar o
/// histórico para CONFERIR o pedido é operação de leitura; a pergunta no grid vivo não
/// mudou e o write vai para o grid vivo).
#[must_use]
pub fn prompt_snapshot_hash(vt: &dyn VtBackend, k: usize) -> String {
    let (cols, rows) = vt.dims();
    // Cursor em coordenadas do grid VIVO: `screen()` devolve o cursor deslocado pelo
    // scroll de leitura (`display_offset`); subtrair o offset recupera a posição real.
    let screen = vt.screen();
    let cursor_line = screen.cursor.line.saturating_sub(screen.display_offset);
    let cursor_col = screen.cursor.col;

    // Últimas `k` linhas não-vazias, varridas do fundo e devolvidas em ordem de viewport.
    let mut lines: Vec<String> = Vec::with_capacity(k);
    for row in (0..rows).rev() {
        if lines.len() >= k {
            break;
        }
        let text = vt.row_text(row);
        let trimmed = text.trim_end();
        if trimmed.is_empty() {
            continue;
        }
        lines.push(trimmed.to_string());
    }
    lines.reverse();

    let mut hasher = Sha256::new();
    hasher.update(format!(
        "dims:{cols}x{rows};cursor:{cursor_line},{cursor_col};k:{k};"
    ));
    for line in &lines {
        hasher.update(line.as_bytes());
        // Separador fora do alfabeto do grid (o emulador não deposita `\n` em célula):
        // "ab"+"c" nunca colide com "a"+"bc".
        hasher.update(b"\n");
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for byte in digest {
        // write! em String é infalível; o `_ =` descarta o Ok obrigatório do trait.
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// Veredito da **Captura 2** (re-snapshot imediatamente antes do write) — núcleo único
/// usado pela porta real ([`PtyHost::deliver_approval`](crate::PtyHost::deliver_approval))
/// e por qualquer porta de teste: uma só fonte para a regra de comparação.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScreenCheck {
    /// A tela ainda mostra a MESMA pergunta — o write está autorizado.
    Match {
        /// O hash conferido (== `expected`); vira o `vt_snapshot_hash` do `ApprovalInjected`.
        vt_snapshot_hash: String,
    },
    /// A tela mudou desde a detecção/aviso — NÃO escrever nenhum byte.
    Changed {
        /// O hash atual (diagnóstico/UI; nunca autoridade).
        current_hash: String,
    },
}

/// Re-snapshota a região do prompt e compara com `expected_hash` (Captura 2, ADR §1).
/// Quem chama DEVE garantir a atomicidade local com o write (segurar o lock do VT que
/// serializa o `advance` — ver `PtyHost::deliver_approval`).
#[must_use]
pub fn check_screen(vt: &dyn VtBackend, expected_hash: &str, k: usize) -> ScreenCheck {
    let current = prompt_snapshot_hash(vt, k);
    if current == expected_hash {
        ScreenCheck::Match {
            vt_snapshot_hash: current,
        }
    } else {
        ScreenCheck::Changed {
            current_hash: current,
        }
    }
}

// ───────────────────────── Porta única de escrita validada ─────────────────────────

/// Resultado da porta de entrega (write validado contra a tela).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortOutcome {
    /// Tela conferida idêntica: `keys` foram escritos no master.
    Written { vt_snapshot_hash: String },
    /// Tela divergiu: NENHUM byte foi escrito.
    ScreenChanged { current_hash: String },
}

/// Erros da porta de entrega.
#[derive(Debug, Error)]
pub enum PortError {
    /// Nenhum terminal sob esse identificador.
    #[error("terminal {0} não encontrado na porta de aprovação")]
    NotFound(String),
    /// Erro de I/O no write (efeito NÃO confirmado — o executor aborta sem `Injected`).
    #[error("erro de I/O na porta de aprovação: {0}")]
    Io(String),
}

/// A **porta única** de escrita de aprovação (ADR 0021 §2): re-snapshot + comparação +
/// write no mesmo turno do loop do pty-host. A impl de produção é
/// [`PtyHost`](crate::PtyHost); impls de teste replicam a regra via [`check_screen`].
pub trait ApprovalPort {
    /// Entrega `keys` ao terminal `node_id` SE (e somente se) a região do prompt ainda
    /// bate com `expected_hash`. `ScreenChanged` ⇒ zero bytes escritos.
    fn deliver(
        &mut self,
        node_id: &str,
        expected_hash: &str,
        keys: &[u8],
    ) -> Result<PortOutcome, PortError>;
}

/// A porta de produção: o pty-host (dono único do PTY — ADR §1 Captura 2). O `node_id`
/// textual é o `NodeId` (UUID) do terminal; o `K` é o default [`PROMPT_REGION_ROWS`].
impl ApprovalPort for crate::PtyHost {
    fn deliver(
        &mut self,
        node_id: &str,
        expected_hash: &str,
        keys: &[u8],
    ) -> Result<PortOutcome, PortError> {
        let node: crate::NodeId = node_id
            .parse()
            .map_err(|_| PortError::NotFound(node_id.to_string()))?;
        self.deliver_approval(node, expected_hash, keys, PROMPT_REGION_ROWS)
            .map_err(|e| match e {
                crate::PtyHostError::NotFound(n) => PortError::NotFound(n.to_string()),
                other => PortError::Io(other.to_string()),
            })
    }
}

// ───────────────────────── §2 · Ledger de idempotência (projeção) ─────────────────────────

/// Estado projetado de um `stable_id` (derivado SÓ de eventos — nunca mutado direto).
#[derive(Debug, Clone, Default)]
struct IdState {
    /// Nó dono do pedido (do `PermissionAsked` — binding de fonte interna, ADR §5).
    node_id: Option<String>,
    /// `PermissionResolved` visto e NÃO reaberto por `ApprovalAborted` (decisão em voo
    /// ou órfã de crash — em ambos os casos, um gesto novo é no-op auditado).
    resolved_open: bool,
    /// `ApprovalInjected` visto — o efeito aconteceu; consumido para sempre.
    delivered: bool,
    /// `PermissionDismissed` visto — "não era um pedido"; nada será escrito.
    dismissed: bool,
    /// `ApprovalDuplicateIgnored` já emitido (anti-amplificação: no máx 1× por pedido).
    duplicate_audited: bool,
}

/// Projeção dos eventos de permissão/aprovação que guia a deduplicação na porta única
/// (ADR 0021 §2). **Derivada do log, reconstruível por replay** — alimente-a com
/// [`ApprovalLedger::observe`] na ordem do log. Não tem acesso a nenhuma porta de
/// escrita: reconstruí-la jamais produz um write (trava por construção).
#[derive(Debug, Default)]
pub struct ApprovalLedger {
    ids: HashMap<String, IdState>,
}

impl ApprovalLedger {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Aplica um evento do log à projeção (reducer puro; eventos alheios são ignorados).
    pub fn observe(&mut self, event: &DomainEvent) {
        match event {
            DomainEvent::PermissionAsked {
                node_id, stable_id, ..
            } => {
                let st = self.ids.entry(stable_id.clone()).or_default();
                st.node_id = Some(node_id.clone());
            }
            DomainEvent::PermissionResolved { stable_id, .. } => {
                self.ids.entry(stable_id.clone()).or_default().resolved_open = true;
            }
            // Abort REABRE o pedido (ADR §1: a UI pede novo gesto) — a decisão abortada
            // não consome o stable_id.
            DomainEvent::ApprovalAborted { stable_id, .. } => {
                self.ids.entry(stable_id.clone()).or_default().resolved_open = false;
            }
            DomainEvent::ApprovalInjected { stable_id, .. } => {
                self.ids.entry(stable_id.clone()).or_default().delivered = true;
            }
            DomainEvent::ApprovalDuplicateIgnored { stable_id } => {
                self.ids
                    .entry(stable_id.clone())
                    .or_default()
                    .duplicate_audited = true;
            }
            DomainEvent::PermissionDismissed { stable_id } => {
                self.ids.entry(stable_id.clone()).or_default().dismissed = true;
            }
            _ => {}
        }
    }

    /// Nó dono do pedido (`None` = pedido desconhecido — sem binding verificável).
    #[must_use]
    pub fn node_of(&self, stable_id: &str) -> Option<&str> {
        self.ids.get(stable_id)?.node_id.as_deref()
    }

    /// `true` se o pedido já foi consumido: entregue (`Injected`), descartado
    /// (`Dismissed`) ou com decisão em voo/órfã (`Resolved` sem abort posterior).
    /// Gesto sobre pedido consumido ⇒ no-op auditado (ADR §2).
    #[must_use]
    pub fn is_consumed(&self, stable_id: &str) -> bool {
        self.ids
            .get(stable_id)
            .is_some_and(|st| st.delivered || st.dismissed || st.resolved_open)
    }

    /// `true` se a duplicata deste pedido JÁ foi auditada (`ApprovalDuplicateIgnored`
    /// emitido) — as seguintes são silenciosas (anti-amplificação, ADR 0003).
    #[must_use]
    pub fn duplicate_audited(&self, stable_id: &str) -> bool {
        self.ids
            .get(stable_id)
            .is_some_and(|st| st.duplicate_audited)
    }
}

// ───────────────────────── Executor (gesto → porta → eventos) ─────────────────────────

/// Um gesto humano FRESCO sobre um pedido de permissão — a única coisa que pode
/// produzir um write (replay/boot não fabricam gestos; ver doutrina do módulo).
#[derive(Debug, Clone, Copy)]
pub struct ApprovalGesture<'a> {
    /// O pedido aprovado/recusado (referencia o `PermissionAsked` — nunca posição de fila).
    pub stable_id: &'a str,
    /// Nó cujo PTY receberá a digitação (cross-check final contra o binding do log — R4).
    pub target_node: &'a str,
    /// A decisão do humano (ou do SLA: deny por timeout — §3, mesmo pipeline validado).
    pub decision: ApprovalDecision,
    /// Via da decisão (`human` = clique; `timeout` = auto-deny do SLA).
    pub via: ResolutionVia,
    /// Captura 1: hash da região do prompt de quando o pedido foi detectado/exibido.
    pub expected_hash: &'a str,
    /// Teclas do CLI Profile do alvo (o executor escolhe approve/deny pela decisão).
    pub keys: &'a ApprovalKeys,
}

/// Razão de um abort (espelha o `reason` em snake_case do evento).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbortReason {
    /// A tela mudou entre a detecção/aviso e o gesto (ADR §1).
    ScreenChanged,
    /// O `stable_id` não pertence ao nó que receberia a digitação (ADR §4 R4) — inclui
    /// pedido desconhecido (sem binding verificável, fail-safe).
    TargetMismatch,
    /// A porta falhou em I/O — efeito não confirmado, auditado sem `Injected`.
    PortError,
}

impl AbortReason {
    /// O `reason` persistido no `ApprovalAborted` (snake_case aberto, contrato do evento).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            AbortReason::ScreenChanged => REASON_SCREEN_CHANGED,
            AbortReason::TargetMismatch => REASON_TARGET_MISMATCH,
            AbortReason::PortError => REASON_PORT_ERROR,
        }
    }
}

/// O que a entrega produziu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalOutcomeKind {
    /// Tela conferida + `approval_keys` digitados (exatamente 1 write).
    Injected,
    /// Nenhum byte escrito; ver [`AbortReason`].
    Aborted(AbortReason),
    /// Pedido já consumido: no-op. `audited == true` na PRIMEIRA duplicata (evento
    /// emitido); `false` nas seguintes (silêncio — anti-amplificação).
    DuplicateIgnored { audited: bool },
}

/// Desfecho da entrega + os eventos a apendar no `EventStore`, **na ordem** (ADR §2:
/// `PermissionResolved` → write → `ApprovalInjected`). O executor já aplicou esses
/// eventos ao próprio ledger; o chamador os persiste no log.
#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalOutcome {
    pub kind: ApprovalOutcomeKind,
    pub events: Vec<DomainEvent>,
}

/// O executor de injeção (ADR 0021 §2): consulta o ledger, confere o alvo, registra a
/// decisão e entrega pela porta única. Dirigido pelo chamador (sem thread/relógio
/// próprio); a serialização dos gestos é do chamador (`&mut self`).
#[derive(Debug, Default)]
pub struct ApprovalExecutor {
    ledger: ApprovalLedger,
}

impl ApprovalExecutor {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Alimenta a projeção com um evento do log (boot/replay/tempo-real). NUNCA escreve
    /// no PTY — não há porta neste caminho (trava por construção).
    pub fn observe(&mut self, event: &DomainEvent) {
        self.ledger.observe(event);
    }

    /// Leitura da projeção (diagnóstico/teste).
    #[must_use]
    pub fn ledger(&self) -> &ApprovalLedger {
        &self.ledger
    }

    /// Entrega a decisão de um gesto humano fresco pela porta única (ADR 0021 §1/§2).
    ///
    /// Sequência: dedup (projeção) → cross-check de alvo (R4) → `PermissionResolved` →
    /// porta (re-snapshot + write atômicos) → `ApprovalInjected` | `ApprovalAborted`.
    /// Os eventos do desfecho já foram aplicados ao ledger interno; o chamador DEVE
    /// apendá-los ao `EventStore` na ordem devolvida.
    pub fn deliver(
        &mut self,
        gesture: &ApprovalGesture<'_>,
        port: &mut dyn ApprovalPort,
    ) -> ApprovalOutcome {
        let stable_id = gesture.stable_id;

        // 1) Idempotência (ADR §2): pedido consumido ⇒ no-op auditado, no máx 1×.
        if self.ledger.is_consumed(stable_id) {
            if self.ledger.duplicate_audited(stable_id) {
                return ApprovalOutcome {
                    kind: ApprovalOutcomeKind::DuplicateIgnored { audited: false },
                    events: vec![],
                };
            }
            let ev = DomainEvent::ApprovalDuplicateIgnored {
                stable_id: stable_id.to_string(),
            };
            self.ledger.observe(&ev);
            return ApprovalOutcome {
                kind: ApprovalOutcomeKind::DuplicateIgnored { audited: true },
                events: vec![ev],
            };
        }

        // 2) Cross-check final de alvo (ADR §4 R4): o binding vem do LOG (fonte interna),
        //    nunca do gesto. Divergência (ou pedido desconhecido) NÃO consome a pendência
        //    nem registra decisão — só audita o abort.
        if self.ledger.node_of(stable_id) != Some(gesture.target_node) {
            let ev = DomainEvent::ApprovalAborted {
                stable_id: stable_id.to_string(),
                reason: AbortReason::TargetMismatch.as_str().to_string(),
            };
            self.ledger.observe(&ev);
            return ApprovalOutcome {
                kind: ApprovalOutcomeKind::Aborted(AbortReason::TargetMismatch),
                events: vec![ev],
            };
        }

        // 3) A decisão é registrada ANTES do efeito (ordem do ADR §2). Observá-la já
        //    fecha a janela de re-entrada: um segundo gesto vira duplicata.
        let resolved = DomainEvent::PermissionResolved {
            stable_id: stable_id.to_string(),
            decision: gesture.decision,
            via: gesture.via,
        };
        self.ledger.observe(&resolved);
        let mut events = vec![resolved];

        // 4) Porta única: re-snapshot + comparação + write no mesmo turno (ADR §1).
        //    O write é SÓ a tecla declarada no profile para ESTA decisão (§3: deny passa
        //    pelo mesmo pipeline validado).
        let keys = match gesture.decision {
            ApprovalDecision::Approve => gesture.keys.approve.as_bytes(),
            ApprovalDecision::Deny => gesture.keys.deny.as_bytes(),
        };
        let (kind, effect) = match port.deliver(gesture.target_node, gesture.expected_hash, keys) {
            Ok(PortOutcome::Written { vt_snapshot_hash }) => (
                ApprovalOutcomeKind::Injected,
                DomainEvent::ApprovalInjected {
                    stable_id: stable_id.to_string(),
                    vt_snapshot_hash,
                },
            ),
            Ok(PortOutcome::ScreenChanged { .. }) => (
                ApprovalOutcomeKind::Aborted(AbortReason::ScreenChanged),
                DomainEvent::ApprovalAborted {
                    stable_id: stable_id.to_string(),
                    reason: AbortReason::ScreenChanged.as_str().to_string(),
                },
            ),
            Err(e) => {
                // Efeito não confirmado: auditar como abort (sem `Injected`) e deixar o
                // humano re-validar visualmente — nunca inventar sucesso.
                tracing::warn!(%stable_id, error = %e, "porta de aprovação falhou em I/O");
                (
                    ApprovalOutcomeKind::Aborted(AbortReason::PortError),
                    DomainEvent::ApprovalAborted {
                        stable_id: stable_id.to_string(),
                        reason: AbortReason::PortError.as_str().to_string(),
                    },
                )
            }
        };
        self.ledger.observe(&effect);
        events.push(effect);
        ApprovalOutcome { kind, events }
    }
}

// ───────────────────────────────────── testes ─────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::PermissionEvidence;
    use lina_vt::AlacrittyBackend;

    const K: usize = PROMPT_REGION_ROWS;

    fn grid_with(bytes: &[u8]) -> AlacrittyBackend {
        let mut vt = AlacrittyBackend::new(80, 24);
        vt.advance(bytes);
        vt
    }

    fn asked(node_id: &str, stable_id: &str) -> DomainEvent {
        DomainEvent::PermissionAsked {
            node_id: node_id.into(),
            tool: None,
            detail: None,
            evidence: PermissionEvidence::Grid,
            stable_id: stable_id.into(),
            vt_snapshot_hash: None,
            prompt_kind: crate::events::PromptKind::Yn,
        }
    }

    // ───────────── §1 · prompt_snapshot_hash (função pura) ─────────────

    /// Determinístico: o mesmo estado de grid produz o mesmo hash (duas chamadas e dois
    /// backends independentes com o mesmo fluxo de bytes); texto novo muda o hash.
    #[test]
    fn hash_is_deterministic_and_text_sensitive() {
        let a = grid_with(b"$ run\r\nContinue? (y/n) ");
        let b = grid_with(b"$ run\r\nContinue? (y/n) ");
        let h1 = prompt_snapshot_hash(&a, K);
        assert_eq!(h1, prompt_snapshot_hash(&a, K), "mesma tela, mesmo hash");
        assert_eq!(
            h1,
            prompt_snapshot_hash(&b, K),
            "grids idênticos, mesmo hash"
        );

        let mut c = grid_with(b"$ run\r\nContinue? (y/n) ");
        c.advance(b"\r\nnew output");
        assert_ne!(h1, prompt_snapshot_hash(&c, K), "texto novo muda o hash");
    }

    /// ADR §1: atributos de cor/estilo ficam FORA do hash — o mesmo texto com SGR
    /// (verde+bold) produz o MESMO hash do texto puro (re-render/tema não abortam).
    #[test]
    fn hash_ignores_color_and_style_attributes() {
        let plain = grid_with(b"Deploy to prod? (y/n) ");
        let styled = grid_with(b"\x1b[1;32mDeploy to prod? (y/n) \x1b[0m");
        assert_eq!(
            prompt_snapshot_hash(&plain, K),
            prompt_snapshot_hash(&styled, K),
            "cor/estilo não entram na semântica do prompt"
        );
    }

    /// A posição do cursor ENTRA no hash: mesmo texto, cursor reposicionado ⇒ difere.
    #[test]
    fn hash_changes_on_cursor_move() {
        let at_end = grid_with(b"Continue? (y/n) ");
        let mut moved = grid_with(b"Continue? (y/n) ");
        moved.advance(b"\x1b[1;1H"); // cursor home, texto intacto
        assert_ne!(
            prompt_snapshot_hash(&at_end, K),
            prompt_snapshot_hash(&moved, K),
            "cursor faz parte da região do prompt"
        );
    }

    /// As dimensões `(cols, rows)` entram no hash (resize re-flowa o prompt).
    #[test]
    fn hash_changes_on_resize() {
        let small = {
            let mut vt = AlacrittyBackend::new(80, 24);
            vt.advance(b"ok? (y/n) ");
            vt
        };
        let wide = {
            let mut vt = AlacrittyBackend::new(100, 24);
            vt.advance(b"ok? (y/n) ");
            vt
        };
        assert_ne!(
            prompt_snapshot_hash(&small, K),
            prompt_snapshot_hash(&wide, K)
        );
    }

    /// `K` delimita a região: mudança numa linha ACIMA da janela de `k` não-vazias não
    /// altera o hash; com `k` maior (janela alcança a linha), altera. E `k` divergente
    /// nunca casa (entra no material do hash).
    #[test]
    fn hash_region_k_limits_lines_and_k_is_part_of_material() {
        // 1 linha antiga divergente + 7 linhas iguais + prompt = a janela k=8 cobre as
        // últimas 8 não-vazias (7 linhas + prompt), deixando a antiga de fora.
        let mut old_a = AlacrittyBackend::new(80, 24);
        old_a.advance(b"old-A\r\n");
        let mut old_b = AlacrittyBackend::new(80, 24);
        old_b.advance(b"old-B\r\n");
        for vt in [&mut old_a, &mut old_b] {
            for i in 0..7 {
                vt.advance(format!("line {i}\r\n").as_bytes());
            }
            vt.advance(b"Continue? (y/n) ");
        }
        assert_eq!(
            prompt_snapshot_hash(&old_a, 8),
            prompt_snapshot_hash(&old_b, 8),
            "linha fora da janela k=8 não participa"
        );
        assert_ne!(
            prompt_snapshot_hash(&old_a, 9),
            prompt_snapshot_hash(&old_b, 9),
            "k=9 alcança a linha divergente"
        );
        assert_ne!(
            prompt_snapshot_hash(&old_a, 8),
            prompt_snapshot_hash(&old_a, 9),
            "K divergente entre capturas nunca casa (fail-safe)"
        );
    }

    /// Scroll de LEITURA (display_offset) fica fora do hash: rolar o histórico para
    /// conferir o pedido não invalida a aprovação — a pergunta no grid vivo não mudou.
    #[test]
    fn hash_is_invariant_to_read_scroll() {
        let mut vt = AlacrittyBackend::new(80, 5);
        for i in 0..30 {
            vt.advance(format!("log {i}\r\n").as_bytes());
        }
        vt.advance(b"Continue? (y/n) ");
        let live = prompt_snapshot_hash(&vt, K);
        vt.scroll(3); // usuário rola 3 linhas para o passado
        assert_eq!(
            live,
            prompt_snapshot_hash(&vt, K),
            "scroll de leitura não muda a pergunta viva"
        );
    }

    // ───────────── Porta fake (write-log de PTY para os ACs) ─────────────

    /// Porta de teste sobre um grid real: aplica a MESMA regra da porta de produção
    /// (via [`check_screen`] — fonte única) e registra cada write (o "log de writes do
    /// PTY" que prova zero/um byte escrito).
    struct FakePort {
        vt: AlacrittyBackend,
        writes: Vec<Vec<u8>>,
    }

    impl FakePort {
        fn new(bytes: &[u8]) -> Self {
            Self {
                vt: grid_with(bytes),
                writes: Vec::new(),
            }
        }

        fn hash(&self) -> String {
            prompt_snapshot_hash(&self.vt, K)
        }
    }

    impl ApprovalPort for FakePort {
        fn deliver(
            &mut self,
            _node_id: &str,
            expected_hash: &str,
            keys: &[u8],
        ) -> Result<PortOutcome, PortError> {
            match check_screen(&self.vt, expected_hash, K) {
                ScreenCheck::Changed { current_hash } => {
                    Ok(PortOutcome::ScreenChanged { current_hash })
                }
                ScreenCheck::Match { vt_snapshot_hash } => {
                    self.writes.push(keys.to_vec());
                    Ok(PortOutcome::Written { vt_snapshot_hash })
                }
            }
        }
    }

    /// Porta que falha em I/O (teste do caminho `port_error`).
    struct ErrPort;
    impl ApprovalPort for ErrPort {
        fn deliver(&mut self, _n: &str, _e: &str, _k: &[u8]) -> Result<PortOutcome, PortError> {
            Err(PortError::Io("writer fechado".into()))
        }
    }

    fn gesture<'a>(
        stable_id: &'a str,
        target: &'a str,
        decision: ApprovalDecision,
        via: ResolutionVia,
        expected_hash: &'a str,
        keys: &'a ApprovalKeys,
    ) -> ApprovalGesture<'a> {
        ApprovalGesture {
            stable_id,
            target_node: target,
            decision,
            via,
            expected_hash,
            keys,
        }
    }

    // ───────────── AC-0021.1 · race: tela mudou ⇒ aborta com ZERO bytes ─────────────

    /// O prompt do alvo muda entre a detecção (`PermissionAsked` + Captura 1) e o gesto
    /// ⇒ `ApprovalAborted{screen_changed}` e ZERO bytes no log de writes do PTY; o
    /// audit trail é `Resolved` + `Aborted`, SEM `Injected` (ADR §2).
    #[test]
    fn ac_0021_1_screen_changed_aborts_with_zero_bytes() {
        let mut port = FakePort::new(b"$ deploy\r\nDeploy to prod? (y/n) ");
        let mut exec = ApprovalExecutor::new();
        exec.observe(&asked("n1", "s1"));
        let captura1 = port.hash(); // hash de quando o aviso foi exibido

        // A tela muda ANTES do clique (output novo do CLI).
        port.vt
            .advance(b"\r\nWARNING: lock file changed\r\nDeploy to prod? (y/n) ");

        let keys = ApprovalKeys::default();
        let out = exec.deliver(
            &gesture(
                "s1",
                "n1",
                ApprovalDecision::Approve,
                ResolutionVia::Human,
                &captura1,
                &keys,
            ),
            &mut port,
        );

        assert_eq!(
            out.kind,
            ApprovalOutcomeKind::Aborted(AbortReason::ScreenChanged)
        );
        assert!(port.writes.is_empty(), "ZERO bytes devem chegar ao PTY");
        assert_eq!(
            out.events,
            vec![
                DomainEvent::PermissionResolved {
                    stable_id: "s1".into(),
                    decision: ApprovalDecision::Approve,
                    via: ResolutionVia::Human,
                },
                DomainEvent::ApprovalAborted {
                    stable_id: "s1".into(),
                    reason: REASON_SCREEN_CHANGED.into(),
                },
            ],
            "auditável como Resolved + Aborted, sem Injected"
        );
    }

    // ───────────── AC-0021.2 · idempotência: aprovar 2× injeta 1× ─────────────

    /// Aprovar 2× o mesmo `stable_id` ⇒ EXATAMENTE 1 write; a 2ª via é no-op auditado
    /// (`ApprovalDuplicateIgnored`, no máximo 1×) e a 3ª é silenciosa.
    #[test]
    fn ac_0021_2_double_approve_injects_exactly_once() {
        let mut port = FakePort::new(b"Continue? (y/n) ");
        let mut exec = ApprovalExecutor::new();
        exec.observe(&asked("n1", "s1"));
        let h = port.hash();
        let keys = ApprovalKeys::default();
        let g = gesture(
            "s1",
            "n1",
            ApprovalDecision::Approve,
            ResolutionVia::Human,
            &h,
            &keys,
        );

        // 1º clique: tela conferida + write das approval_keys + audit trail completo.
        let first = exec.deliver(&g, &mut port);
        assert_eq!(first.kind, ApprovalOutcomeKind::Injected);
        assert_eq!(
            port.writes,
            vec![b"y\r".to_vec()],
            "1 write, só approval_keys"
        );
        assert_eq!(
            first.events[1],
            DomainEvent::ApprovalInjected {
                stable_id: "s1".into(),
                vt_snapshot_hash: h.clone(),
            },
            "o Injected carrega o hash conferido"
        );

        // 2º clique (duplicata): no-op AUDITADO — nenhum write novo.
        let second = exec.deliver(&g, &mut port);
        assert_eq!(
            second.kind,
            ApprovalOutcomeKind::DuplicateIgnored { audited: true }
        );
        assert_eq!(
            second.events,
            vec![DomainEvent::ApprovalDuplicateIgnored {
                stable_id: "s1".into()
            }]
        );

        // 3º clique: silencioso (anti-amplificação — no máx 1 evento de duplicata).
        let third = exec.deliver(&g, &mut port);
        assert_eq!(
            third.kind,
            ApprovalOutcomeKind::DuplicateIgnored { audited: false }
        );
        assert!(third.events.is_empty());

        assert_eq!(port.writes.len(), 1, "EXATAMENTE 1 digitação no total");
    }

    // ───────────── R4 · cross-check de alvo ─────────────

    /// `stable_id` de um nó entregue contra o PTY de OUTRO ⇒ `target_mismatch`, zero
    /// bytes, e a pendência fica INTACTA (sem Resolved) — o gesto correto ainda entrega.
    #[test]
    fn target_mismatch_aborts_without_consuming_the_request() {
        let mut port = FakePort::new(b"Continue? (y/n) ");
        let mut exec = ApprovalExecutor::new();
        exec.observe(&asked("n1", "s1"));
        let h = port.hash();
        let keys = ApprovalKeys::default();

        let wrong = exec.deliver(
            &gesture(
                "s1",
                "n2",
                ApprovalDecision::Approve,
                ResolutionVia::Human,
                &h,
                &keys,
            ),
            &mut port,
        );
        assert_eq!(
            wrong.kind,
            ApprovalOutcomeKind::Aborted(AbortReason::TargetMismatch)
        );
        assert!(port.writes.is_empty());
        assert_eq!(
            wrong.events,
            vec![DomainEvent::ApprovalAborted {
                stable_id: "s1".into(),
                reason: REASON_TARGET_MISMATCH.into(),
            }],
            "mismatch não registra decisão (pendência intacta)"
        );

        // O gesto CORRETO (mesmo stable_id, nó dono) ainda entrega — nada foi consumido.
        let right = exec.deliver(
            &gesture(
                "s1",
                "n1",
                ApprovalDecision::Approve,
                ResolutionVia::Human,
                &h,
                &keys,
            ),
            &mut port,
        );
        assert_eq!(right.kind, ApprovalOutcomeKind::Injected);
        assert_eq!(port.writes.len(), 1);
    }

    /// Pedido DESCONHECIDO (sem `PermissionAsked` no log) não tem binding verificável ⇒
    /// `target_mismatch` fail-safe, zero bytes.
    #[test]
    fn unknown_stable_id_aborts_as_target_mismatch() {
        let mut port = FakePort::new(b"Continue? (y/n) ");
        let mut exec = ApprovalExecutor::new();
        let h = port.hash();
        let keys = ApprovalKeys::default();
        let out = exec.deliver(
            &gesture(
                "ghost",
                "n1",
                ApprovalDecision::Approve,
                ResolutionVia::Human,
                &h,
                &keys,
            ),
            &mut port,
        );
        assert_eq!(
            out.kind,
            ApprovalOutcomeKind::Aborted(AbortReason::TargetMismatch)
        );
        assert!(port.writes.is_empty());
    }

    // ───────────── §3 · deny pelo MESMO pipeline validado ─────────────

    /// Recusa (inclusive o auto-deny do SLA, `via=timeout`) digita as teclas de DENY do
    /// profile pelo mesmo pipeline: tela válida ⇒ escreve `n\r`; tela mudada ⇒ aborta
    /// com zero bytes (simetria total do §3).
    #[test]
    fn deny_uses_deny_keys_through_the_same_validated_pipeline() {
        let keys = ApprovalKeys::default();

        // Tela válida: escreve EXATAMENTE as teclas de deny.
        let mut port = FakePort::new(b"Continue? (y/n) ");
        let mut exec = ApprovalExecutor::new();
        exec.observe(&asked("n1", "s1"));
        let h = port.hash();
        let out = exec.deliver(
            &gesture(
                "s1",
                "n1",
                ApprovalDecision::Deny,
                ResolutionVia::Timeout,
                &h,
                &keys,
            ),
            &mut port,
        );
        assert_eq!(out.kind, ApprovalOutcomeKind::Injected);
        assert_eq!(port.writes, vec![b"n\r".to_vec()]);
        assert_eq!(
            out.events[0],
            DomainEvent::PermissionResolved {
                stable_id: "s1".into(),
                decision: ApprovalDecision::Deny,
                via: ResolutionVia::Timeout,
            },
            "auto-deny do SLA auditado como deny/timeout"
        );

        // Tela mudada: deny-não-entregue — aborta sem escrever (rótulo honesto na UI).
        let mut port2 = FakePort::new(b"Continue? (y/n) ");
        let mut exec2 = ApprovalExecutor::new();
        exec2.observe(&asked("n1", "s2"));
        let stale = port2.hash();
        port2.vt.advance(b"\r\nmore output");
        let out2 = exec2.deliver(
            &gesture(
                "s2",
                "n1",
                ApprovalDecision::Deny,
                ResolutionVia::Timeout,
                &stale,
                &keys,
            ),
            &mut port2,
        );
        assert_eq!(
            out2.kind,
            ApprovalOutcomeKind::Aborted(AbortReason::ScreenChanged)
        );
        assert!(port2.writes.is_empty());
    }

    // ───────────── §2 · reinício nunca re-digita; abort reabre ─────────────

    /// Ledger RECONSTRUÍDO do log (boot/replay): pedido já entregue ⇒ gesto retardatário
    /// é no-op; pedido com `Resolved` órfão de crash (sem `Injected`) TAMBÉM não
    /// re-digita — nenhum caminho de replay produz write (o rebuild nem tem porta).
    #[test]
    fn rebuild_from_log_never_rewrites() {
        let keys = ApprovalKeys::default();

        // Caso 1: log completo (Asked → Resolved → Injected). Replay + gesto ⇒ no-op.
        let mut port = FakePort::new(b"Continue? (y/n) ");
        let h = port.hash();
        let mut exec = ApprovalExecutor::new();
        for ev in [
            asked("n1", "s1"),
            DomainEvent::PermissionResolved {
                stable_id: "s1".into(),
                decision: ApprovalDecision::Approve,
                via: ResolutionVia::Human,
            },
            DomainEvent::ApprovalInjected {
                stable_id: "s1".into(),
                vt_snapshot_hash: h.clone(),
            },
        ] {
            exec.observe(&ev); // boot: replay do log — sem porta, sem write possível
        }
        let out = exec.deliver(
            &gesture(
                "s1",
                "n1",
                ApprovalDecision::Approve,
                ResolutionVia::Human,
                &h,
                &keys,
            ),
            &mut port,
        );
        assert_eq!(
            out.kind,
            ApprovalOutcomeKind::DuplicateIgnored { audited: true }
        );
        assert!(port.writes.is_empty(), "replay + gesto velho: zero writes");

        // Caso 2: crash entre Resolved e o write (log SEM Injected) ⇒ igualmente no-op;
        // o prompt persistente será RE-detectado com stable_id novo (ADR §2).
        let mut exec2 = ApprovalExecutor::new();
        exec2.observe(&asked("n1", "s2"));
        exec2.observe(&DomainEvent::PermissionResolved {
            stable_id: "s2".into(),
            decision: ApprovalDecision::Approve,
            via: ResolutionVia::Human,
        });
        let out2 = exec2.deliver(
            &gesture(
                "s2",
                "n1",
                ApprovalDecision::Approve,
                ResolutionVia::Human,
                &h,
                &keys,
            ),
            &mut port,
        );
        assert_eq!(
            out2.kind,
            ApprovalOutcomeKind::DuplicateIgnored { audited: true }
        );
        assert!(port.writes.is_empty());
    }

    /// `ApprovalAborted` REABRE o pedido (ADR §1: a UI pede novo gesto): após um abort
    /// por tela mudada, um gesto fresco com o hash ATUAL entrega normalmente.
    #[test]
    fn reclick_after_abort_delivers() {
        let mut port = FakePort::new(b"Continue? (y/n) ");
        let mut exec = ApprovalExecutor::new();
        exec.observe(&asked("n1", "s1"));
        let keys = ApprovalKeys::default();

        let stale = port.hash();
        port.vt.advance(b"\r\nextra line\r\nContinue? (y/n) ");
        let aborted = exec.deliver(
            &gesture(
                "s1",
                "n1",
                ApprovalDecision::Approve,
                ResolutionVia::Human,
                &stale,
                &keys,
            ),
            &mut port,
        );
        assert_eq!(
            aborted.kind,
            ApprovalOutcomeKind::Aborted(AbortReason::ScreenChanged)
        );
        assert!(port.writes.is_empty());

        // Novo clique, hash da tela ATUAL (a UI reapresentou o estado): entrega.
        let fresh = port.hash();
        let ok = exec.deliver(
            &gesture(
                "s1",
                "n1",
                ApprovalDecision::Approve,
                ResolutionVia::Human,
                &fresh,
                &keys,
            ),
            &mut port,
        );
        assert_eq!(ok.kind, ApprovalOutcomeKind::Injected);
        assert_eq!(port.writes, vec![b"y\r".to_vec()]);
    }

    /// A auditoria de duplicata sobrevive ao replay: log que JÁ contém o
    /// `ApprovalDuplicateIgnored` ⇒ a próxima duplicata é silenciosa (máx 1× POR PEDIDO,
    /// mesmo cruzando restart — anti-amplificação ADR 0003).
    #[test]
    fn duplicate_audit_is_at_most_once_across_rebuild() {
        let mut port = FakePort::new(b"Continue? (y/n) ");
        let h = port.hash();
        let keys = ApprovalKeys::default();
        let mut exec = ApprovalExecutor::new();
        for ev in [
            asked("n1", "s1"),
            DomainEvent::ApprovalInjected {
                stable_id: "s1".into(),
                vt_snapshot_hash: h.clone(),
            },
            DomainEvent::ApprovalDuplicateIgnored {
                stable_id: "s1".into(),
            },
        ] {
            exec.observe(&ev);
        }
        let out = exec.deliver(
            &gesture(
                "s1",
                "n1",
                ApprovalDecision::Approve,
                ResolutionVia::Human,
                &h,
                &keys,
            ),
            &mut port,
        );
        assert_eq!(
            out.kind,
            ApprovalOutcomeKind::DuplicateIgnored { audited: false }
        );
        assert!(out.events.is_empty(), "sem 2º evento de duplicata");
        assert!(port.writes.is_empty());
    }

    /// `PermissionDismissed` ("não era um pedido") consome a pendência: gesto posterior
    /// é no-op auditado, zero bytes.
    #[test]
    fn dismissed_request_never_writes() {
        let mut port = FakePort::new(b"Continue? (y/n) ");
        let h = port.hash();
        let keys = ApprovalKeys::default();
        let mut exec = ApprovalExecutor::new();
        exec.observe(&asked("n1", "s1"));
        exec.observe(&DomainEvent::PermissionDismissed {
            stable_id: "s1".into(),
        });
        let out = exec.deliver(
            &gesture(
                "s1",
                "n1",
                ApprovalDecision::Approve,
                ResolutionVia::Human,
                &h,
                &keys,
            ),
            &mut port,
        );
        assert_eq!(
            out.kind,
            ApprovalOutcomeKind::DuplicateIgnored { audited: true }
        );
        assert!(port.writes.is_empty());
    }

    /// Erro de I/O na porta: efeito NÃO confirmado ⇒ auditado como `Resolved` +
    /// `Aborted{port_error}`, sem `Injected` — nunca inventa sucesso.
    #[test]
    fn port_io_error_audits_abort_without_injected() {
        let mut exec = ApprovalExecutor::new();
        exec.observe(&asked("n1", "s1"));
        let keys = ApprovalKeys::default();
        let out = exec.deliver(
            &gesture(
                "s1",
                "n1",
                ApprovalDecision::Approve,
                ResolutionVia::Human,
                "h",
                &keys,
            ),
            &mut ErrPort,
        );
        assert_eq!(
            out.kind,
            ApprovalOutcomeKind::Aborted(AbortReason::PortError)
        );
        assert_eq!(
            out.events[1],
            DomainEvent::ApprovalAborted {
                stable_id: "s1".into(),
                reason: REASON_PORT_ERROR.into(),
            }
        );
    }

    // ───────────── Porta REAL (PtyHost): mesmo turno do loop, PTY vivo ─────────────

    /// Integração com o pty-host real (a porta de produção): um processo bloqueado num
    /// prompt y/n real; (1) hash de DETECÇÃO capturado; (2) a tela muda (humano digita);
    /// entrega com o hash velho ⇒ `ScreenChanged` e o `read` segue bloqueado (nenhum
    /// byte chegou); (3) entrega com hash FRESCO ⇒ `Written` e o programa destrava com
    /// EXATAMENTE o input esperado — se o abort tivesse vazado bytes, o `read` teria
    /// consumido outra coisa e este passo falharia (prova positiva do zero-byte).
    #[cfg(unix)] // F1-6-8: spawna `sh -c` em PTY real — runtime-Unix (tabela ci-3so-triagem.md).
    #[test]
    fn pty_port_screen_changed_writes_nothing_then_fresh_hash_delivers() {
        use std::time::{Duration, Instant};

        fn poll_until(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
            let start = Instant::now();
            loop {
                if cond() {
                    return true;
                }
                if start.elapsed() >= timeout {
                    return false;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        const T: Duration = Duration::from_secs(5);

        let mut host = crate::PtyHost::new();
        let node = host
            .spawn(
                crate::PtyCommand::new("sh")
                    .arg("-c")
                    .arg("printf 'Continue? (y/n) '; read x; printf 'GOT:%s\\n' \"$x\""),
                80,
                24,
            )
            .expect("spawn do prompt sintético");

        // O prompt aparece no grid parseado (condição, não sleep).
        assert!(
            poll_until(T, || host
                .with_grid(node, |vt| vt.last_nonempty_line())
                .as_deref()
                == Some("Continue? (y/n)")),
            "o prompt deveria aparecer no grid"
        );

        // Captura 1 (detecção/aviso).
        let captura1 = host
            .with_grid(node, |vt| prompt_snapshot_hash(vt, PROMPT_REGION_ROWS))
            .expect("hash da detecção");

        // A tela MUDA entre o aviso e o clique: o humano digita um caractere no
        // terminal (o eco altera a região do prompt).
        host.write(node, b"x").expect("write humano");
        assert!(
            poll_until(T, || host
                .with_grid(node, |vt| vt.last_nonempty_line())
                .as_deref()
                == Some("Continue? (y/n) x")),
            "o eco do 'x' deveria mudar a tela"
        );

        // Clique com o hash VELHO ⇒ ScreenChanged, nenhum byte escrito.
        let stale = host
            .deliver_approval(node, &captura1, b"y\r", PROMPT_REGION_ROWS)
            .expect("porta deve responder");
        assert!(
            matches!(stale, PortOutcome::ScreenChanged { .. }),
            "tela mudada deve abortar: {stale:?}"
        );

        // Novo gesto com o hash FRESCO ⇒ Written; o read destrava com "xy" — a prova de
        // que o abort não escreveu nada (senão o read teria consumido antes/diferente).
        let captura2 = host
            .with_grid(node, |vt| prompt_snapshot_hash(vt, PROMPT_REGION_ROWS))
            .expect("hash fresco");
        let ok = host
            .deliver_approval(node, &captura2, b"y\r", PROMPT_REGION_ROWS)
            .expect("porta deve responder");
        assert!(
            matches!(ok, PortOutcome::Written { .. }),
            "tela válida escreve: {ok:?}"
        );
        assert!(
            poll_until(T, || host
                .with_grid(node, |vt| vt.last_nonempty_line())
                .as_deref()
                == Some("GOT:xy")),
            "o programa deveria destravar com EXATAMENTE o input 'xy'"
        );

        host.kill(node).ok();
    }

    /// Porta real: nó inexistente erra limpo (sem panic).
    #[test]
    fn pty_port_missing_node_errors_cleanly() {
        let host = crate::PtyHost::new();
        let ghost = uuid::Uuid::now_v7();
        assert!(matches!(
            host.deliver_approval(ghost, "h", b"y\r", PROMPT_REGION_ROWS),
            Err(crate::PtyHostError::NotFound(_))
        ));
    }
}
