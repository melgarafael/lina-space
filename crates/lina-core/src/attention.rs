//! F1-1-7 · Fila de atenção UNIFICADA (custódia + permissão) — projeção do event log.
//!
//! ## Arquitetura (story F1-1-7; ADR 0021 §3/§6; fonte 13.13 achados 4/6)
//! - **Uma fila só**, com precedência **custódia > permissão** (13.13 achado 4; a
//!   custódia é o gate duro do ADR 0004 — nunca fica atrás de um y/n) e **drain justo
//!   round-robin por nó** dentro de cada classe (um nó que flooda não enterra o pedido
//!   único de outro — zero starvation, provado por teste headless).
//! - **Event-sourced de verdade:** o estado SÓ muda via [`AttentionQueue::observe`]
//!   (fold de [`DomainEvent`]). [`AttentionQueue::resolve`]/[`AttentionQueue::dismiss`]
//!   são **comandos puros**: validam contra o estado e devolvem o evento a apendar —
//!   o chamador apenda no [`EventStore`](crate::events::EventStore) e re-alimenta o
//!   `observe`. Replay ≡ live **por construção** (crash com pendência → reabrir
//!   reconstrói via [`AttentionQueue::replay`]).
//! - **Dedup entre camadas (decisão do Maestro):** o fallback de grid fica ativo
//!   também para CLIs com hook — o grid detecta em ~1,3s e a `Notification` chega
//!   ~5,8s depois (latência intrínseca medida na F1-1-6). O item NASCE da primeira
//!   camada e a segunda **enriquece o MESMO item** (janela
//!   [`LAYER_MERGE_WINDOW_MS`] por nó; ver [`AttentionQueue::observe`]).
//! - **Mitigação de FP é produto (decisão do Maestro):** botão "não era um pedido"
//!   ([`AttentionQueue::dismiss`] → `PermissionDismissed`, alimenta a telemetria do
//!   detector) e allowlist por nó ([`AttentionQueue::set_node_muted`] →
//!   `NodeDetectionMuted`, persistido e reversível — último evento do nó vence).
//!
//! ## Fronteira (ADR 0021 §6 — inegociável)
//! Esta projeção **mostra, enfileira, audita e decide** — e NADA mais. O estado
//! `Escalated` aos 5 min é **VISUAL** (badge/alerta). **Nenhum caminho daqui escreve
//! no PTY**: o write do y/n (inclusive o auto-deny do SLA aos 10 min) é exclusivo do
//! executor da F1-1-8, atrás do ADR 0021. A custódia continua inteira no caminho
//! existente (`broker::run_custody` + pump do app — zero regressão): a fila apenas
//! ESPELHA as pendências de custódia para ordenação/exibição unificada
//! ([`AttentionQueue::custody_enqueued`]/[`AttentionQueue::custody_resolved`]); a
//! durabilidade delas é a fila de broker em disco (re-drain repõe pós-crash) e a
//! auditoria é a já existente (`ActionGated`/`BrokerExecuted`/`BrokerDenied`).

use std::collections::HashMap;

/// Re-export para o consumidor de UI (`lina_core::attention::PromptKind`): o shell
/// decide a apresentação por formato — `Yn` rende toast aprovável; `Choice`/`Trust`
/// rendem alerta + foco no terminal, SEM aprovar/recusar (direção do fundador R2b).
/// (Re-export daqui, e não do lib.rs, porque o lib.rs está em obra de outro dono
/// nesta rodada — costura coordenada.)
pub use crate::events::PromptKind;
use crate::events::{
    ApprovalDecision, DomainEvent, EventRecord, PermissionEvidence, ResolutionVia,
};

/// SLA de escalação VISUAL (ADR 0021 §3): pendência sem resposta há ≥ 5 min vira
/// [`AttentionState::Escalated`] (badge pulsante/entrada persistente na UI). O
/// auto-deny aos 10 min NÃO é daqui (write no PTY = F1-1-8).
pub const ESCALATE_AFTER_MS: u64 = 300_000;

/// Janela de merge entre camadas POR NÓ: um `PermissionAsked` de camada DIFERENTE da
/// de um item ainda pendente do mesmo nó, chegando dentro desta janela, é o MESMO
/// pedido físico (enriquece, não duplica). Calibre: a `Notification` do Claude Code
/// chega ~5,8s (estável, medido na F1-1-6) depois do diálogo visível no grid —
/// 10s cobre com margem. Curta o bastante para não fundir pedidos distintos: um
/// prompt y/n BLOQUEIA o CLI no PTY, então um segundo pedido real do mesmo nó dentro
/// da janela exigiria o primeiro ainda não-respondido — não acontece num TUI serial.
pub const LAYER_MERGE_WINDOW_MS: u64 = 10_000;

/// Classe do item na fila — define a precedência (custódia > permissão).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttentionKind {
    /// Gate duro de custódia/resume (`lina do`/`lina resume`, ADR 0004) — espelhado
    /// da pump do app. `Resume` entra como `Custody` (mesmo canal humano W3-6).
    Custody,
    /// Pedido de permissão y/n detectado (F1-1-6).
    Permission,
}

/// Camada de origem da evidência — confiabilidade NÃO-uniforme por design: `Hook` é
/// estrutural; `Grid` é heurística (FP medido, #28174); `Custody` é o canal brokerado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttentionEvidence {
    Hook,
    Grid,
    Custody,
}

impl From<PermissionEvidence> for AttentionEvidence {
    fn from(e: PermissionEvidence) -> Self {
        match e {
            PermissionEvidence::Hook => Self::Hook,
            PermissionEvidence::Grid => Self::Grid,
        }
    }
}

/// Estado VISUAL do item (computado de `now_ms` em [`AttentionQueue::items`] — puro,
/// relógio injetado; nada de timer interno).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttentionState {
    Pending,
    /// ≥ [`ESCALATE_AFTER_MS`] sem resposta (ADR 0021 §3 — escalação aos 5 min).
    Escalated,
}

/// Um item da fila unificada — o CONTRATO com a UI (toast/fila/badge). `detail` é
/// dado de EXIBIÇÃO (do `PreToolUse` correlacionado ou da linha do prompt) — jamais
/// autoridade (doutrina ADR 0021 §5: o gesto referencia `stable_id`, nunca posição
/// nem texto).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttentionItem {
    pub stable_id: String,
    pub node_id: String,
    pub kind: AttentionKind,
    /// Ex.: `"git push origin/master"` (hook) ou a linha do prompt (grid) ou o
    /// `display` da custódia.
    pub detail: Option<String>,
    pub evidence: AttentionEvidence,
    pub created_ts: u64,
    pub state: AttentionState,
    /// R2b: formato do bloqueante — a UI apresenta por ele: `Yn` = toast aprovável;
    /// `Choice`/`Trust` = alerta + foco, sem aprovar/recusar (e [`AttentionQueue::resolve`]
    /// devolve `None` para não-`Yn`, defesa em profundidade). Custódia carrega `Yn`
    /// nominal (o campo não se aplica — o gate dela é o ⌘⏎ da pump).
    pub prompt_kind: PromptKind,
    /// R2b (ADR 0021 §1): a Captura 1 do detector — é o que torna a fila reconstruída
    /// por replay APROVÁVEL (o gesto passa este hash ao `deliver_approval` como
    /// `expected_hash`). `None` = origem sem captura (hook) ou item de custódia.
    pub vt_snapshot_hash: Option<String>,
}

/// Pendência de permissão no fold (com os aliases do merge entre camadas).
#[derive(Debug, Clone)]
struct PendingPermission {
    stable_id: String,
    node_id: String,
    detail: Option<String>,
    evidence: PermissionEvidence,
    created_ts: u64,
    /// `stable_id`s ABSORVIDOS pelo merge entre camadas — resolver por qualquer um
    /// resolve o item; o evento sai sempre com o canônico (`stable_id`).
    aliases: Vec<String>,
    /// Formato do bloqueante (R2b). No merge, só a camada de GRID atualiza — é ela
    /// que VÊ a forma visual do prompt; o hook não sabe o formato.
    prompt_kind: PromptKind,
    /// Captura 1 (R2b) — preenche-se se ausente no merge (o grid é quem captura).
    vt_snapshot_hash: Option<String>,
}

impl PendingPermission {
    fn matches(&self, id: &str) -> bool {
        self.stable_id == id || self.aliases.iter().any(|a| a == id)
    }
}

/// Pendência de custódia ESPELHADA da pump do app (ver §Fronteira no topo).
#[derive(Debug, Clone)]
struct PendingCustody {
    id: String,
    node_id: String,
    display: String,
    created_ts: u64,
}

/// A fila de atenção unificada — uma por workspace. Ver doc do módulo.
#[derive(Debug, Default)]
pub struct AttentionQueue {
    /// Pendências de permissão, em ordem de chegada (= ordem de `created_ts`).
    permissions: Vec<PendingPermission>,
    /// Pendências de custódia espelhadas, em ordem de chegada.
    custody: Vec<PendingCustody>,
    /// Allowlist por nó (`NodeDetectionMuted`, último vence): `true` = fallback de
    /// grid DESLIGADO para o nó.
    muted: HashMap<String, bool>,
}

impl AttentionQueue {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// **O fold** — único caminho de mutação por evento (replay ≡ live). `ts` é o
    /// timestamp do registro (`EventRecord::ts` no replay; o `now_ms` do append ao
    /// vivo).
    pub fn observe(&mut self, event: &DomainEvent, ts: u64) {
        match event {
            DomainEvent::PermissionAsked {
                node_id,
                detail,
                evidence,
                stable_id,
                vt_snapshot_hash,
                prompt_kind,
                ..
            } => self.fold_asked(
                node_id,
                detail.clone(),
                *evidence,
                stable_id,
                vt_snapshot_hash.clone(),
                *prompt_kind,
                ts,
            ),
            // Remoção da pendência: decisão (Resolved), rótulo de FP (Dismissed) ou
            // prompt respondido DIRETO no terminal (PromptCleared, R2b item 5 — sem
            // decisão fabricada; `matches` cobre canônico E aliases do merge).
            DomainEvent::PermissionResolved { stable_id, .. }
            | DomainEvent::PermissionDismissed { stable_id }
            | DomainEvent::PermissionPromptCleared { stable_id, .. } => {
                self.permissions.retain(|p| !p.matches(stable_id));
            }
            DomainEvent::NodeDetectionMuted { node_id, muted } => {
                self.muted.insert(node_id.clone(), *muted);
                if *muted {
                    // Mutar = o humano marcou a família de FP daquele nó: as pendências
                    // de GRID dele saem da fila (determinístico no replay — o próprio
                    // evento de mute está no log). Itens com evidência de HOOK ficam
                    // (camada estrutural, não-heurística — não é alvo do mute).
                    self.permissions.retain(|p| {
                        p.node_id != *node_id || p.evidence != PermissionEvidence::Grid
                    });
                }
            }
            // Pós-decisão do executor (F1-1-8): sem efeito na fila — o item já saiu
            // no `PermissionResolved`. Um abort (`screen_changed`) reapresenta via
            // RE-DETECÇÃO (novo `PermissionAsked`, novo stable_id — ADR 0021 §1).
            _ => {}
        }
    }

    /// Reconstrói a fila varrendo o log (crash com pendência → reabrir reconstrói —
    /// critério da story). Registros não-decodificáveis (kind de versão futura) são
    /// pulados: a fila é projeção derivada, não o validador do log (isso é o
    /// `project()` do store).
    #[must_use]
    pub fn replay(records: &[EventRecord]) -> Self {
        let mut q = Self::new();
        for rec in records {
            if let Ok(ev) = DomainEvent::from_record(&rec.kind, rec.version, rec.payload.clone()) {
                q.observe(&ev, rec.ts);
            }
        }
        q
    }

    #[allow(clippy::too_many_arguments)] // espelho 1:1 dos campos do evento (fold)
    fn fold_asked(
        &mut self,
        node_id: &str,
        detail: Option<String>,
        evidence: PermissionEvidence,
        stable_id: &str,
        vt_snapshot_hash: Option<String>,
        prompt_kind: PromptKind,
        ts: u64,
    ) {
        // Defesa em profundidade: nó mutado não acumula pendência de GRID (o chamador
        // idealmente nem roda `observe_grid` — ver `is_node_muted` —, mas o replay de
        // um log com asks pós-mute também precisa convergir).
        if evidence == PermissionEvidence::Grid && self.is_node_muted(node_id) {
            return;
        }
        // Replay defensivo: o mesmo stable_id (canônico ou alias) não re-entra.
        if self.permissions.iter().any(|p| p.matches(stable_id)) {
            return;
        }
        // Dedup ENTRE CAMADAS (decisão do Maestro): pendência do MESMO nó, de camada
        // OPOSTA, dentro da janela → mesmo pedido físico; enriquece o item existente.
        if let Some(existing) = self.permissions.iter_mut().find(|p| {
            p.node_id == node_id
                && p.evidence != evidence
                && ts.saturating_sub(p.created_ts) <= LAYER_MERGE_WINDOW_MS
        }) {
            if evidence == PermissionEvidence::Hook {
                // Hook chegando depois do grid (caminho dominante: ~1,3s vs ~5,8s):
                // promove a evidência (estrutural > heurística) e prefere o detail do
                // hook (comando real correlacionado, ex.: "git push origin/master",
                // contra a linha crua do prompt).
                existing.evidence = PermissionEvidence::Hook;
                if detail.is_some() {
                    existing.detail = detail;
                }
            } else {
                // Grid chegando (antes OU depois do hook): é a camada que VÊ o prompt —
                // atualiza o formato do bloqueante e preenche exibição ausente.
                existing.prompt_kind = prompt_kind;
                if existing.detail.is_none() {
                    existing.detail = detail;
                }
            }
            // Captura 1: preenche se ausente (o grid é quem captura; hook traz None).
            if existing.vt_snapshot_hash.is_none() {
                existing.vt_snapshot_hash = vt_snapshot_hash;
            }
            existing.aliases.push(stable_id.to_string());
            return;
        }
        self.permissions.push(PendingPermission {
            stable_id: stable_id.to_string(),
            node_id: node_id.to_string(),
            detail,
            evidence,
            created_ts: ts,
            aliases: Vec::new(),
            prompt_kind,
            vt_snapshot_hash,
        });
    }

    // ───────────────────────── espelho da custódia (pump do app) ─────────────────────────

    /// A pump enfileirou um gate de custódia/resume (`PendingGate`): espelha aqui para
    /// a ordenação unificada. `id` = id da `MailMessage` (a chave que a pump já usa);
    /// `node_id` = requester AUTENTICADO (dir-dono do drain — nunca campo de payload).
    /// Idempotente por `id` (re-drain pós-crash não duplica o espelho).
    pub fn custody_enqueued(
        &mut self,
        id: impl Into<String>,
        node_id: impl Into<String>,
        display: impl Into<String>,
        ts: u64,
    ) {
        let id = id.into();
        if self.custody.iter().any(|c| c.id == id) {
            return;
        }
        self.custody.push(PendingCustody {
            id,
            node_id: node_id.into(),
            display: display.into(),
            created_ts: ts,
        });
    }

    /// A pump resolveu (confirmou/recusou) o gate de custódia `id` — sai do espelho.
    /// A auditoria do desfecho é a já existente do broker (`ActionGated`/
    /// `BrokerExecuted`/`BrokerDenied`) — zero mudança no caminho da custódia.
    pub fn custody_resolved(&mut self, id: &str) {
        self.custody.retain(|c| c.id != id);
    }

    // ───────────────────────── comandos (puros: estado → evento) ─────────────────────────

    /// Comando de decisão humana sobre uma PERMISSÃO pendente. Devolve o
    /// `PermissionResolved` a entregar ao executor (com o `stable_id` CANÔNICO, mesmo
    /// se chamado por um alias do merge) — ou `None` se: id desconhecido/já resolvido
    /// (idempotência da fila: clique duplo não gera segundo evento); item de CUSTÓDIA
    /// (a custódia decide pelo caminho existente da pump — zero regressão); ou
    /// **`prompt_kind` não-`Yn`** (R2b: `Choice`/`Trust` alertam + focam, sem
    /// aprovar/recusar — injetar `y` numa caixa de escolha seria input errado; defesa
    /// em profundidade além do gate de apresentação da UI).
    #[must_use]
    pub fn resolve(
        &self,
        stable_id: &str,
        decision: ApprovalDecision,
        via: ResolutionVia,
    ) -> Option<DomainEvent> {
        let item = self.permissions.iter().find(|p| p.matches(stable_id))?;
        if item.prompt_kind != PromptKind::Yn {
            return None;
        }
        Some(DomainEvent::PermissionResolved {
            stable_id: item.stable_id.clone(),
            decision,
            via,
        })
    }

    /// Comando "não era um pedido" (mitigação de FP como produto). Devolve o
    /// `PermissionDismissed` a apendar (stable_id canônico) — `None` se
    /// desconhecido/já saiu. NÃO é deny: nada será escrito nem negado ao agente; o
    /// chamador também alimenta `PermissionDetector::record_false_positive` (rótulo
    /// externo — nunca auto-inferido, #28174).
    #[must_use]
    pub fn dismiss(&self, stable_id: &str) -> Option<DomainEvent> {
        let item = self.permissions.iter().find(|p| p.matches(stable_id))?;
        Some(DomainEvent::PermissionDismissed {
            stable_id: item.stable_id.clone(),
        })
    }

    /// Comando da allowlist por nó: liga/desliga o fallback de GRID para `node_id`.
    /// Devolve o evento a apendar (persistido e REVERSÍVEL — último evento vence).
    #[must_use]
    pub fn set_node_muted(&self, node_id: impl Into<String>, muted: bool) -> DomainEvent {
        DomainEvent::NodeDetectionMuted {
            node_id: node_id.into(),
            muted,
        }
    }

    /// `true` se o fallback de grid está DESLIGADO para o nó — o loop de detecção
    /// consulta antes de chamar `PermissionDetector::observe_grid` (e o fold também
    /// barra, defensivamente).
    #[must_use]
    pub fn is_node_muted(&self, node_id: &str) -> bool {
        self.muted.get(node_id).copied().unwrap_or(false)
    }

    // ───────────────────────────── leitura (contrato com a UI) ─────────────────────────────

    /// A fila ORDENADA para exibição/drain: precedência **custódia > permissão**;
    /// dentro de cada classe, **round-robin por nó** (rodada 0 = item mais antigo de
    /// cada nó, na ordem do nó mais antigo; depois rodada 1; …) — um nó com N
    /// pendências não enterra o pedido único de outro (zero starvation). `state` é
    /// computado de `now_ms` (puro): ≥ 5 min sem resposta → `Escalated` (visual).
    #[must_use]
    pub fn items(&self, now_ms: u64) -> Vec<AttentionItem> {
        let state_of = |created: u64| {
            if now_ms.saturating_sub(created) >= ESCALATE_AFTER_MS {
                AttentionState::Escalated
            } else {
                AttentionState::Pending
            }
        };
        let custody = self.custody.iter().map(|c| AttentionItem {
            stable_id: c.id.clone(),
            node_id: c.node_id.clone(),
            kind: AttentionKind::Custody,
            detail: Some(c.display.clone()),
            evidence: AttentionEvidence::Custody,
            created_ts: c.created_ts,
            state: state_of(c.created_ts),
            prompt_kind: PromptKind::Yn, // nominal: não se aplica à custódia (gate ⌘⏎)
            vt_snapshot_hash: None,
        });
        let perms = self.permissions.iter().map(|p| AttentionItem {
            stable_id: p.stable_id.clone(),
            node_id: p.node_id.clone(),
            kind: AttentionKind::Permission,
            detail: p.detail.clone(),
            evidence: p.evidence.into(),
            created_ts: p.created_ts,
            state: state_of(p.created_ts),
            prompt_kind: p.prompt_kind,
            vt_snapshot_hash: p.vt_snapshot_hash.clone(),
        });
        let mut out = round_robin_by_node(custody.collect());
        out.extend(round_robin_by_node(perms.collect()));
        out
    }

    /// Contagem de pendências (badge da UI: custódia + permissão).
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.custody.len() + self.permissions.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pending_count() == 0
    }
}

/// Intercala itens (já em ordem de chegada) em rodadas round-robin por nó:
/// rodada k = o (k+1)-ésimo item de cada nó, nós na ordem do seu item mais antigo.
/// Determinístico e sem cursor mutável — a mesma fila produz sempre a mesma ordem
/// (replay-friendly; a justiça é provada por teste, não por agendador).
fn round_robin_by_node(items: Vec<AttentionItem>) -> Vec<AttentionItem> {
    // Agrupa preservando ordem de chegada (itens já chegam por created_ts).
    let mut nodes: Vec<(String, Vec<AttentionItem>)> = Vec::new();
    for item in items {
        match nodes.iter_mut().find(|(n, _)| *n == item.node_id) {
            Some((_, bucket)) => bucket.push(item),
            None => nodes.push((item.node_id.clone(), vec![item])),
        }
    }
    let mut out = Vec::new();
    let mut round = 0_usize;
    loop {
        let mut emitted = false;
        for (_, bucket) in &nodes {
            if let Some(item) = bucket.get(round) {
                out.push(item.clone());
                emitted = true;
            }
        }
        if !emitted {
            return out;
        }
        round += 1;
    }
}

// ───────────────────────────────────── testes ─────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventStore;
    use serial_test::serial;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    const T0: u64 = 1_750_000_000_000;

    fn ask(node: &str, evidence: PermissionEvidence, stable_id: &str) -> DomainEvent {
        DomainEvent::PermissionAsked {
            node_id: node.into(),
            tool: None,
            detail: Some(format!("detail-{stable_id}")),
            evidence,
            stable_id: stable_id.into(),
            vt_snapshot_hash: None,
            prompt_kind: PromptKind::Yn,
        }
    }

    /// `ask` com formato + Captura 1 (R2b) — para os testes de prompt_kind/hash.
    fn ask_kind(
        node: &str,
        evidence: PermissionEvidence,
        stable_id: &str,
        prompt_kind: PromptKind,
        vt_snapshot_hash: Option<&str>,
    ) -> DomainEvent {
        DomainEvent::PermissionAsked {
            node_id: node.into(),
            tool: None,
            detail: Some(format!("detail-{stable_id}")),
            evidence,
            stable_id: stable_id.into(),
            vt_snapshot_hash: vt_snapshot_hash.map(str::to_string),
            prompt_kind,
        }
    }

    /// Critério da story (teste headless do round-robin): nó A flooda 3 pedidos; o
    /// pedido ÚNICO de B (posterior a todos) sai em 2º — alternância por nó, zero
    /// starvation. A ordem completa prova a justiça.
    #[test]
    fn round_robin_no_starvation_under_flood() {
        let mut q = AttentionQueue::new();
        q.observe(&ask("A", PermissionEvidence::Grid, "a1"), T0);
        // Fora da janela de merge (mesma camada nunca funde; janelas distintas provam
        // que são pedidos independentes do mesmo nó).
        q.observe(&ask("A", PermissionEvidence::Grid, "a2"), T0 + 20_000);
        q.observe(&ask("A", PermissionEvidence::Grid, "a3"), T0 + 40_000);
        q.observe(&ask("B", PermissionEvidence::Grid, "b1"), T0 + 60_000);
        let order: Vec<String> = q
            .items(T0 + 61_000)
            .into_iter()
            .map(|i| i.stable_id)
            .collect();
        assert_eq!(
            order,
            vec!["a1", "b1", "a2", "a3"],
            "rodada 0 = mais antigo de CADA nó (B não espera o flood de A)"
        );
    }

    /// Precedência: custódia chega DEPOIS, mas sai ANTES (custódia > permissão —
    /// ADR 0004 é o gate duro; 13.13 achado 4).
    #[test]
    fn custody_precedes_permission_regardless_of_arrival() {
        let mut q = AttentionQueue::new();
        q.observe(&ask("A", PermissionEvidence::Hook, "p1"), T0);
        q.custody_enqueued("msg_c1", "B", "lina do deploy [deploy/prod]", T0 + 5_000);
        let items = q.items(T0 + 6_000);
        assert_eq!(items[0].stable_id, "msg_c1");
        assert_eq!(items[0].kind, AttentionKind::Custody);
        assert_eq!(items[0].evidence, AttentionEvidence::Custody);
        assert_eq!(items[1].stable_id, "p1");
        // Espelho idempotente (re-drain pós-crash) e resolução remove.
        q.custody_enqueued("msg_c1", "B", "lina do deploy [deploy/prod]", T0 + 7_000);
        assert_eq!(q.pending_count(), 2);
        q.custody_resolved("msg_c1");
        assert_eq!(q.items(T0 + 8_000).len(), 1);
    }

    /// Dedup entre camadas (decisão do Maestro): item nasce do GRID (~1,3s), hook
    /// chega ~5,8s depois → MESMO item, evidência promovida a hook, detail do hook
    /// (comando real); resolver por QUALQUER dos dois ids emite o canônico (grid).
    #[test]
    fn hook_enriches_grid_born_item_within_window() {
        let mut q = AttentionQueue::new();
        q.observe(&ask("A", PermissionEvidence::Grid, "g1"), T0);
        q.observe(
            &DomainEvent::PermissionAsked {
                node_id: "A".into(),
                tool: Some("Bash".into()),
                detail: Some("git push origin/master".into()),
                evidence: PermissionEvidence::Hook,
                stable_id: "h1".into(),
                vt_snapshot_hash: None,
                prompt_kind: PromptKind::Yn,
            },
            T0 + 5_800,
        );
        let items = q.items(T0 + 6_000);
        assert_eq!(items.len(), 1, "1 pedido físico = 1 item (não 2)");
        assert_eq!(items[0].stable_id, "g1", "canônico = quem nasceu primeiro");
        assert_eq!(items[0].evidence, AttentionEvidence::Hook, "promovida");
        assert_eq!(items[0].detail.as_deref(), Some("git push origin/master"));

        // Resolver pelo ALIAS (h1) emite o evento com o canônico (g1).
        let ev = q
            .resolve("h1", ApprovalDecision::Approve, ResolutionVia::Human)
            .expect("alias resolve");
        match &ev {
            DomainEvent::PermissionResolved { stable_id, .. } => assert_eq!(stable_id, "g1"),
            other => panic!("evento errado: {other:?}"),
        }
        q.observe(&ev, T0 + 7_000);
        assert!(q.is_empty(), "resolver via alias remove o item inteiro");
    }

    /// R2b: item `Choice` expõe formato + Captura 1 ao consumidor de UI; `resolve`
    /// devolve `None` (alerta + foco, sem aprovar/recusar — injetar `y` numa caixa de
    /// escolha seria input errado); `dismiss` (botão de FP) continua funcionando.
    #[test]
    fn choice_item_exposes_kind_and_is_not_resolvable() {
        let mut q = AttentionQueue::new();
        q.observe(
            &ask_kind(
                "A",
                PermissionEvidence::Grid,
                "c1",
                PromptKind::Choice,
                Some("cap1-xyz"),
            ),
            T0,
        );
        let items = q.items(T0 + 1_000);
        assert_eq!(items[0].prompt_kind, PromptKind::Choice);
        assert_eq!(items[0].vt_snapshot_hash.as_deref(), Some("cap1-xyz"));
        assert!(
            q.resolve("c1", ApprovalDecision::Approve, ResolutionVia::Human)
                .is_none(),
            "Choice não é aprovável pela fila (defesa além do gate da UI)"
        );
        assert!(
            q.resolve("c1", ApprovalDecision::Deny, ResolutionVia::Timeout)
                .is_none(),
            "nem recusável — o caminho é alerta + foco no terminal"
        );
        assert!(
            q.dismiss("c1").is_some(),
            "FP dismiss vale p/ qualquer formato"
        );
    }

    /// R2b: no merge entre camadas, o GRID é a fonte do formato (é quem VÊ o prompt) —
    /// hook posterior não rebaixa `Choice` p/ `Yn`; grid posterior atualiza o formato
    /// de item nascido do hook; a Captura 1 preenche-se quando ausente.
    #[test]
    fn merge_grid_owns_prompt_kind_and_fills_capture() {
        // Grid-born Choice + cap1; hook (Yn, sem captura) enriquece SEM rebaixar.
        let mut q = AttentionQueue::new();
        q.observe(
            &ask_kind(
                "A",
                PermissionEvidence::Grid,
                "g1",
                PromptKind::Choice,
                Some("cap1-a"),
            ),
            T0,
        );
        q.observe(&ask("A", PermissionEvidence::Hook, "h1"), T0 + 5_000);
        let items = q.items(T0 + 6_000);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].prompt_kind, PromptKind::Choice, "hook não rebaixa");
        assert_eq!(items[0].vt_snapshot_hash.as_deref(), Some("cap1-a"));

        // Hook-born (Yn, sem captura); grid posterior traz formato + Captura 1.
        let mut q2 = AttentionQueue::new();
        q2.observe(&ask("B", PermissionEvidence::Hook, "h2"), T0);
        q2.observe(
            &ask_kind(
                "B",
                PermissionEvidence::Grid,
                "g2",
                PromptKind::Trust,
                Some("cap1-b"),
            ),
            T0 + 2_000,
        );
        let items2 = q2.items(T0 + 3_000);
        assert_eq!(items2.len(), 1);
        assert_eq!(items2[0].stable_id, "h2", "canônico = quem nasceu primeiro");
        assert_eq!(
            items2[0].prompt_kind,
            PromptKind::Trust,
            "grid define o formato"
        );
        assert_eq!(items2[0].vt_snapshot_hash.as_deref(), Some("cap1-b"));
    }

    /// R2b item 5: prompt respondido DIRETO no terminal (`PermissionPromptCleared`)
    /// remove a pendência — inclusive via ALIAS do merge (o detector emite o id do
    /// lado grid; o item pode ter canônico hook-born). Id desconhecido = no-op.
    /// Sem decisão fabricada: nenhum Resolved/Dismissed no fluxo.
    #[test]
    fn prompt_cleared_removes_item_including_via_alias() {
        let mut q = AttentionQueue::new();
        q.observe(&ask("A", PermissionEvidence::Hook, "h1"), T0);
        q.observe(
            &ask_kind(
                "A",
                PermissionEvidence::Grid,
                "g1",
                PromptKind::Yn,
                Some("cap"),
            ),
            T0 + 2_000,
        );
        assert_eq!(q.pending_count(), 1, "merge: 1 pedido físico");
        q.observe(
            &DomainEvent::PermissionPromptCleared {
                node_id: "A".into(),
                stable_id: "g1".into(),
            },
            T0 + 9_000,
        );
        assert!(q.is_empty(), "cleared via alias remove o item inteiro");

        // Id desconhecido: no-op (replay defensivo).
        q.observe(
            &DomainEvent::PermissionPromptCleared {
                node_id: "A".into(),
                stable_id: "nope".into(),
            },
            T0 + 10_000,
        );
        assert!(q.is_empty());
    }

    /// Fora da janela de merge → pedidos DISTINTOS (não funde).
    #[test]
    fn cross_layer_outside_window_stays_two_items() {
        let mut q = AttentionQueue::new();
        q.observe(&ask("A", PermissionEvidence::Grid, "g1"), T0);
        q.observe(
            &ask("A", PermissionEvidence::Hook, "h1"),
            T0 + LAYER_MERGE_WINDOW_MS + 1,
        );
        assert_eq!(q.items(T0 + 20_000).len(), 2);
    }

    /// Critério da story: decisão remove da fila + evento correlacionado ao
    /// stable_id; segunda via do MESMO id → `None` (idempotência da fila — clique
    /// duplo não gera segundo evento).
    #[test]
    fn resolve_removes_and_is_idempotent() {
        let mut q = AttentionQueue::new();
        q.observe(&ask("A", PermissionEvidence::Hook, "p1"), T0);
        let ev = q
            .resolve("p1", ApprovalDecision::Deny, ResolutionVia::Human)
            .expect("resolve 1");
        q.observe(&ev, T0 + 100);
        assert!(q.is_empty());
        assert!(
            q.resolve("p1", ApprovalDecision::Deny, ResolutionVia::Human)
                .is_none(),
            "já resolvido → None"
        );
        assert!(q.dismiss("p1").is_none(), "já saiu → None");
    }

    /// Botão "não era um pedido": `dismiss` → `PermissionDismissed` → observe remove.
    #[test]
    fn dismiss_emits_and_removes() {
        let mut q = AttentionQueue::new();
        q.observe(&ask("A", PermissionEvidence::Grid, "p1"), T0);
        let ev = q.dismiss("p1").expect("dismiss");
        match &ev {
            DomainEvent::PermissionDismissed { stable_id } => assert_eq!(stable_id, "p1"),
            other => panic!("evento errado: {other:?}"),
        }
        q.observe(&ev, T0 + 100);
        assert!(q.is_empty());
    }

    /// Allowlist por nó: mute derruba as pendências de GRID do nó (família de FP),
    /// preserva as de HOOK (estrutural), barra asks de grid novos, e o unmute
    /// (muted:false — REVERSÍVEL, último vence) re-habilita.
    #[test]
    fn mute_drops_grid_keeps_hook_and_is_reversible() {
        let mut q = AttentionQueue::new();
        q.observe(&ask("A", PermissionEvidence::Grid, "g1"), T0);
        q.observe(&ask("A", PermissionEvidence::Hook, "h1"), T0 + 60_000);
        q.observe(&ask("B", PermissionEvidence::Grid, "gb"), T0 + 60_000);

        let mute = q.set_node_muted("A", true);
        q.observe(&mute, T0 + 61_000);
        assert!(q.is_node_muted("A"));
        let ids: Vec<String> = q
            .items(T0 + 62_000)
            .into_iter()
            .map(|i| i.stable_id)
            .collect();
        assert!(!ids.contains(&"g1".to_string()), "grid do nó mutado sai");
        assert!(ids.contains(&"h1".to_string()), "hook do nó mutado FICA");
        assert!(ids.contains(&"gb".to_string()), "outro nó intocado");

        // Ask de grid novo do nó mutado não entra (defensivo no fold).
        q.observe(&ask("A", PermissionEvidence::Grid, "g2"), T0 + 63_000);
        assert_eq!(q.pending_count(), 2);

        // Reversível: unmute re-habilita.
        let unmute = q.set_node_muted("A", false);
        q.observe(&unmute, T0 + 64_000);
        assert!(!q.is_node_muted("A"));
        q.observe(&ask("A", PermissionEvidence::Grid, "g3"), T0 + 80_000);
        assert_eq!(q.pending_count(), 3);
    }

    /// Escalação VISUAL aos 5 min (ADR 0021 §3): puro de `now_ms` — antes Pending,
    /// depois Escalated. NENHUM write/auto-deny nasce daqui (fronteira F1-1-8).
    #[test]
    fn escalates_visually_at_five_minutes() {
        let mut q = AttentionQueue::new();
        q.observe(&ask("A", PermissionEvidence::Hook, "p1"), T0);
        assert_eq!(
            q.items(T0 + ESCALATE_AFTER_MS - 1)[0].state,
            AttentionState::Pending
        );
        assert_eq!(
            q.items(T0 + ESCALATE_AFTER_MS)[0].state,
            AttentionState::Escalated
        );
    }

    /// Replay defensivo: o MESMO stable_id duas vezes no log (não deveria, mas o log
    /// é histórico) não duplica o item.
    #[test]
    fn duplicate_stable_id_in_log_does_not_duplicate_item() {
        let mut q = AttentionQueue::new();
        q.observe(&ask("A", PermissionEvidence::Grid, "p1"), T0);
        q.observe(&ask("A", PermissionEvidence::Grid, "p1"), T0 + 100);
        assert_eq!(q.pending_count(), 1);
    }

    // ───────────── replay contra um EventStore real (critério da story) ─────────────

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let p = std::env::temp_dir()
                .join(format!("lina-f117-{tag}-{}-{nanos}", std::process::id()));
            std::fs::create_dir_all(&p).expect("criar tempdir");
            Self(p)
        }
        fn path(&self) -> &Path {
            self.0.as_ref()
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Critério da story: crash com pendência → reabrir reconstrói a fila do log.
    /// 3 asks + 1 resolved + 1 dismissed no store; `replay` devolve SÓ a pendência
    /// viva, idêntica à da fila que viveu o fluxo (replay ≡ live).
    #[test]
    #[serial]
    fn replay_from_store_rebuilds_pending_queue() {
        let tmp = TempDir::new("replay");
        let mut store = EventStore::open(tmp.path()).expect("open");
        let mut live = AttentionQueue::new();

        let evs = [
            ask("A", PermissionEvidence::Hook, "p1"),
            ask("B", PermissionEvidence::Grid, "p2"),
            ask("C", PermissionEvidence::Hook, "p3"),
        ];
        for ev in &evs {
            store.append(ev).expect("append ask");
        }
        let resolve = DomainEvent::PermissionResolved {
            stable_id: "p1".into(),
            decision: ApprovalDecision::Approve,
            via: ResolutionVia::Human,
        };
        store.append(&resolve).expect("append resolved");
        let dismiss = DomainEvent::PermissionDismissed {
            stable_id: "p3".into(),
        };
        store.append(&dismiss).expect("append dismissed");

        // A fila viva consome os MESMOS fatos com os ts persistidos.
        let records = store.events().expect("events");
        for rec in &records {
            if let Ok(ev) = DomainEvent::from_record(&rec.kind, rec.version, rec.payload.clone()) {
                live.observe(&ev, rec.ts);
            }
        }

        // "Crash + reabrir": reconstrói do log e compara com a fila que viveu.
        let rebuilt = AttentionQueue::replay(&records);
        let now = records.last().map(|r| r.ts).unwrap_or(T0);
        assert_eq!(rebuilt.items(now), live.items(now), "replay ≡ live");
        let ids: Vec<String> = rebuilt
            .items(now)
            .into_iter()
            .map(|i| i.stable_id)
            .collect();
        assert_eq!(ids, vec!["p2"], "só a pendência não-resolvida sobrevive");
    }
}
