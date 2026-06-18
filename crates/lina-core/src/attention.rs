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
/// [`AttentionState::Escalated`] (badge pulsante/entrada persistente na UI). O auto-deny
/// aos 10 min é IDENTIFICADO aqui ([`AttentionQueue::auto_deny_due`]), mas o WRITE da
/// recusa no PTY continua sendo do executor F1-1-8 — esta projeção não tem porta de
/// escrita (fronteira intacta).
pub const ESCALATE_AFTER_MS: u64 = 300_000;

/// SLA de **auto-deny** (ADR 0021 §3): pendência de PERMISSÃO sem resposta há ≥ 10 min é
/// NEGADA automaticamente (`decision=deny, via=timeout` — **NUNCA approve, sem knob**;
/// anti-regressão pelo gate AC-0021.5). DISTINTO do `RETENTION_TIMEOUT_MS` do router (que
/// rege a retenção de mensagem A2A, não a permissão). 10 min = mesma ordem de grandeza da
/// atenção humana num workspace vivo (ADR 0020: turnos reais 200–600 s; `retention_timeout`
/// = 10 min). [`AttentionQueue::auto_deny_due`] só IDENTIFICA as pendências vencidas; o
/// write da recusa (com check de tela) é do executor — esta projeção nunca escreve no PTY.
pub const AUTO_DENY_AFTER_MS: u64 = 600_000;

// Invariante de COMPILAÇÃO (ADR 0021 §3) — escalate-before-auto-deny: a escalação VISUAL
// precede o auto-deny, i.e. o pedido alerta o humano (5 min) ANTES de ser negado por
// timeout (10 min). Recalibrar uma constante para violar a ordem QUEBRA O BUILD, nunca
// silenciosamente o produto.
const _: () = assert!(ESCALATE_AFTER_MS < AUTO_DENY_AFTER_MS);

/// Janela de merge entre camadas POR NÓ: um `PermissionAsked` de camada DIFERENTE da
/// de um item ainda pendente do mesmo nó, chegando dentro desta janela, é o MESMO
/// pedido físico (enriquece, não duplica). Calibre: a `Notification` do Claude Code
/// chega ~5,8s (estável, medido na F1-1-6) depois do diálogo visível no grid —
/// 10s cobre com margem. Curta o bastante para não fundir pedidos distintos: um
/// prompt y/n BLOQUEIA o CLI no PTY, então um segundo pedido real do mesmo nó dentro
/// da janela exigiria o primeiro ainda não-respondido — não acontece num TUI serial.
pub const LAYER_MERGE_WINDOW_MS: u64 = 10_000;

/// FIX-2 (dogfood) — **TTL do ask do guard** (hook `PreToolUse`). Diferente do y/n de permissão, o
/// hook-ask NÃO emite um evento de resolução quando respondido no terminal, e no core não há sinal
/// determinístico de "respondido" (o nó fica `Busy` desenhando o próprio dialog — sinal ambíguo;
/// `recolhe-no-Busy` é follow-up de app). Então o item some por TEMPO desde o ÚLTIMO ask do nó
/// (renovável). 3 min: generoso para a resposta humana típica (ADR 0020: turnos 200–600 s),
/// priorizando VISIBILIDADE (direção do fundador: nenhum bloqueio invisível) e abaixo de
/// [`ESCALATE_AFTER_MS`] (5 min). **Limitação conhecida (v1):** se o humano demora > TTL num único
/// ask, o item some antes de responder; e pode persistir até o TTL após já respondido.
pub const GUARD_ASK_TTL_MS: u64 = 180_000;

/// Classe do item na fila — define a precedência (custódia > permissão > spawn > guard-ask).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttentionKind {
    /// Gate duro de custódia/resume (`lina do`/`lina resume`, ADR 0004) — espelhado
    /// da pump do app. `Resume` entra como `Custody` (mesmo canal humano W3-6).
    Custody,
    /// Pedido de permissão y/n detectado (F1-1-6).
    Permission,
    /// SEAM-1 (ADR 0019 §6): um agente em CASCATA pediu criar um terminal (`SpawnGated{cascade}`) —
    /// gate humano (anti-fork-bomb). Aprovar = o terminal nasce (1×, dedupe durável M3); recusar =
    /// nada nasce. Precedência ABAIXO de permissão (criar processo pode esperar; um bloqueante de
    /// permissão trava um turno em curso).
    Spawn,
    /// FIX-2 (dogfood): um ASK do guard do Lina (hook `PreToolUse` → `ActionGated{decision:"ask"}`)
    /// que BLOQUEIA o agente, mas o detector de permissão (F1-1-6) não pega — o dialog de hook-ask
    /// tem outro formato que o nativo (e em `bypassPermissions` o nativo nem existe). Era o maior
    /// buraco de "bloqueio invisível ao usuário". Resolução v1: ALERTA + FOCA o terminal (sem
    /// injeção remota — espelha a decisão do fundador p/ não-`Yn`: o gesto resolve NO terminal).
    /// Precedência mais baixa (responder y/n no terminal pode esperar atrás dos gates que travam um
    /// turno), mas NUNCA invisível (direção do fundador 2026-06-07: todo bloqueante alerta).
    GuardAsk,
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

/// Uma pendência VENCIDA pelo SLA de auto-deny (ADR 0021 §3) — o que
/// [`AttentionQueue::auto_deny_due`] entrega ao chamador para drenar pelo MESMO pipeline
/// validado do executor com `decision=Deny, via=Timeout`. **NÃO é um write** nem um
/// evento: é a IDENTIFICAÇÃO da pendência; o write (com re-snapshot + check de tela) é
/// exclusivo do executor (fronteira F1-1-8). Por NÃO carregar decisão, nenhum caminho
/// daqui pode produzir approve (a decisão é fixada `Deny` no ponto de drenagem).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoDenyDue {
    /// `stable_id` CANÔNICO da pendência (o executor cruza com o binding do log — R4).
    pub stable_id: String,
    /// Nó dono (do `PermissionAsked` — binding de fonte interna; o executor re-verifica).
    pub node_id: String,
    /// Captura 1 (vira o `expected_hash` do check de tela do executor). `None` = origem
    /// sem captura (hook-only): o executor não tem baseline ⇒ aborta fail-safe
    /// (deny-não-entregue), nunca escreve às cegas.
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

/// SEAM-1: pendência de SPAWN gateado (cascata) aguardando o gate humano. Reconstruída do log
/// (`SpawnRequested` traz name/role/requested_by; `SpawnGated{cascade}` promove a pendente) — então
/// um banner pendente SOBREVIVE a crash/reabrir (replay). `requested_by` é `String` (NodeId
/// serializado), como os demais ids da fila; o app re-lê o `SpawnRequested` TIPADO ao executar.
#[derive(Debug, Clone)]
struct PendingSpawn {
    /// `= SpawnRequested.id` (== `root_cause_id` na origem) — a chave do gesto/dedupe.
    id: String,
    /// Quem pediu (sender AUTENTICADO do gate), para exibição/identidade.
    requested_by: String,
    /// `@Nome` do terminal pedido.
    name: String,
    /// Papel pedido.
    role: String,
    created_ts: u64,
}

/// FIX-2: pendência de um ASK do guard (hook `PreToolUse`), POR NÓ. Dedup por nó: o guard dispara
/// 1×/tool gated — um item só; o último ask renova `last_ts`/`cmd`. Some por TTL
/// ([`GUARD_ASK_TTL_MS`]) desde `last_ts` (resolução v1 por timestamp).
#[derive(Debug, Clone)]
struct PendingGuardAsk {
    /// NOME do terminal (`LINA_NODE_NAME`) — o foco da fila é POR NOME (`attention_goto_node`).
    node: String,
    /// Resumo do comando barrado (copy leiga "vá ao terminal aprovar X"); o último ask vence.
    cmd: String,
    /// 1ª vez visto (idade exibida; estável no replay).
    created_ts: u64,
    /// Último ask do nó — renova o TTL (dedup N→1).
    last_ts: u64,
}

/// A fila de atenção unificada — uma por workspace. Ver doc do módulo.
#[derive(Debug, Default)]
pub struct AttentionQueue {
    /// Pendências de permissão, em ordem de chegada (= ordem de `created_ts`).
    permissions: Vec<PendingPermission>,
    /// Pendências de custódia espelhadas, em ordem de chegada.
    custody: Vec<PendingCustody>,
    /// SEAM-1: spawns gateados (cascata) aguardando o gate humano (banner). Pendente = `SpawnGated
    /// {cascade}` sem `SpawnAdmitted`/`SpawnDeclined` posterior.
    spawns: Vec<PendingSpawn>,
    /// SEAM-1: detalhes de TODO `SpawnRequested` visto (id → (requested_by, name, role, ts)) — o
    /// `SpawnGated{cascade}` posterior promove a pendente usando-os (os dois são apendados em
    /// sequência por `handle_spawn`). Removido quando o spawn resolve (admitido/recusado).
    spawn_requests: HashMap<String, (String, String, String, u64)>,
    /// FIX-2: asks do guard (hook `PreToolUse`) pendentes, um por nó (dedup N→1). Em ordem de
    /// chegada (= `created_ts`); somem por TTL no [`AttentionQueue::items`].
    guard_asks: Vec<PendingGuardAsk>,
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
            // SEAM-1: captura os args de TODO pedido de spawn (precede o gate). Origem (que vira
            // SpawnApproved, sem SpawnGated) nunca é promovida a banner; só a CASCATA abaixo.
            DomainEvent::SpawnRequested {
                id,
                requested_by,
                name,
                role,
                ..
            } => {
                self.spawn_requests.insert(
                    id.clone(),
                    (requested_by.to_string(), name.clone(), role.clone(), ts),
                );
            }
            // SEAM-1: só a CASCATA vira banner (gate humano anti-fork-bomb). Outros motivos
            // (manual/over_cap/cost) são recusas/limites, não pedidos human-gated nesta rodada.
            DomainEvent::SpawnGated { id, reason, .. } if reason == "cascade" => {
                if self.spawns.iter().any(|s| s.id == *id) {
                    return; // replay defensivo: não re-entra
                }
                if let Some((requested_by, name, role, req_ts)) = self.spawn_requests.get(id) {
                    self.spawns.push(PendingSpawn {
                        id: id.clone(),
                        requested_by: requested_by.clone(),
                        name: name.clone(),
                        role: role.clone(),
                        created_ts: *req_ts,
                    });
                }
            }
            // SEAM-1: spawn resolvido (admitido pelo app OU recusado pelo humano) → sai do banner.
            DomainEvent::SpawnAdmitted { id, .. } | DomainEvent::SpawnDeclined { id } => {
                self.spawns.retain(|s| s.id != *id);
                self.spawn_requests.remove(id);
            }
            // FIX-2: ASK do guard (hook PreToolUse) COM nó identificado → item GuardAsk (alerta+foco).
            // SÓ `decision=="ask"` E `node:Some` entram: `deny`/`allow` são terminais/silenciosos (não
            // bloqueiam o humano) e `node:None` é custódia/log-antigo (sem terminal para focar — a
            // custódia já tem item próprio). Dedup por NÓ no fold (N→1).
            DomainEvent::ActionGated {
                decision,
                cmd,
                node: Some(node),
                ..
            } if decision == "ask" => self.fold_guard_ask(node, cmd, ts),
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
        q.observe_records(records);
        q
    }

    /// Aplica eventos do log (já em ordem de `seq`) à fila VIVA — base da projeção INCREMENTAL.
    /// `replay` é `new()` + `observe_records(todos)`; aplicar SÓ os registros novos
    /// (`seq > último aplicado`) à fila persistida converge ao MESMO estado, pois `observe` é um
    /// fold sequencial determinístico. É o que tira o full-replay `O(N)` da thread de UI no
    /// `AttentionHub::sync` (mesmo padrão do cache incremental de `EventStore::project`).
    pub fn observe_records(&mut self, records: &[EventRecord]) {
        for rec in records {
            if let Ok(ev) = DomainEvent::from_record(&rec.kind, rec.version, rec.payload.clone()) {
                self.observe(&ev, rec.ts);
            }
        }
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

    /// FIX-2: funde um ASK do guard no item do nó (dedup N→1: renova `last_ts`/`cmd`; o 1º fixa
    /// `created_ts`). O guard pode disparar uma vez por tool gated — o mesmo nó nunca empilha dois
    /// itens. Determinístico no replay (eventos em ordem de `ts`).
    fn fold_guard_ask(&mut self, node: &str, cmd: &str, ts: u64) {
        if let Some(existing) = self.guard_asks.iter_mut().find(|g| g.node == node) {
            existing.last_ts = ts;
            existing.cmd = cmd.to_string();
            return;
        }
        self.guard_asks.push(PendingGuardAsk {
            node: node.to_string(),
            cmd: cmd.to_string(),
            created_ts: ts,
            last_ts: ts,
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
        // SEAM-1: banners de spawn (cascata) — precedência ABAIXO de permissão. `detail` = copy leiga
        // ("trazer um especialista de <role>"); a UI enriquece com o nome do solicitante (model).
        let spawns = self.spawns.iter().map(|s| AttentionItem {
            stable_id: s.id.clone(),
            node_id: s.requested_by.clone(),
            kind: AttentionKind::Spawn,
            detail: Some(format!("trazer um especialista de {} ({})", s.role, s.name)),
            evidence: AttentionEvidence::Custody, // request estruturado (não-heurístico), como custódia
            created_ts: s.created_ts,
            state: state_of(s.created_ts),
            prompt_kind: PromptKind::Yn, // y/n: deixar nascer? (gate humano)
            vt_snapshot_hash: None,
        });
        // FIX-2: asks do guard (hook PreToolUse) — precedência ABAIXO de spawn. SOMEM por TTL desde o
        // ÚLTIMO ask (`last_ts`): resolução v1 por timestamp. `detail` = comando barrado (copy leiga;
        // a UI enriquece com o nome). `prompt_kind = Choice`: alerta + foco, NUNCA aprovável daqui
        // (defesa em profundidade — `resolve`/`auto_deny_due` só olham as permissões `Yn`).
        let guard = self
            .guard_asks
            .iter()
            .filter(|g| now_ms.saturating_sub(g.last_ts) < GUARD_ASK_TTL_MS)
            .map(|g| AttentionItem {
                stable_id: format!("guard:{}", g.node),
                node_id: g.node.clone(),
                kind: AttentionKind::GuardAsk,
                detail: Some(g.cmd.clone()),
                evidence: AttentionEvidence::Hook, // ask ESTRUTURAL do gate (não-heurístico)
                created_ts: g.created_ts,
                state: state_of(g.created_ts),
                prompt_kind: PromptKind::Choice,
                vt_snapshot_hash: None,
            });
        let mut out = round_robin_by_node(custody.collect());
        out.extend(round_robin_by_node(perms.collect()));
        out.extend(round_robin_by_node(spawns.collect()));
        out.extend(round_robin_by_node(guard.collect()));
        out
    }

    /// SEAM-1: comando de RECUSA de um banner de spawn pendente. Devolve o `SpawnDeclined` a apendar
    /// (`None` se id desconhecido/já resolvido — idempotência). A APROVAÇÃO NÃO mora aqui: criar o
    /// terminal é do app (`admit_node` + `SpawnAdmitted`, fora do core); o app ramifica por
    /// `kind == Spawn` e re-lê o `SpawnRequested` TIPADO do log para executar.
    #[must_use]
    pub fn decline_spawn(&self, id: &str) -> Option<DomainEvent> {
        self.spawns.iter().find(|s| s.id == id)?;
        Some(DomainEvent::SpawnDeclined { id: id.to_string() })
    }

    /// SEAM-1: `true` se `id` é um banner de spawn PENDENTE (o app ramifica o gesto: aprovar → admite;
    /// recusar → [`AttentionQueue::decline_spawn`]).
    #[must_use]
    pub fn is_pending_spawn(&self, id: &str) -> bool {
        self.spawns.iter().any(|s| s.id == id)
    }

    /// **Driver de auto-deny (ADR 0021 §3)** — as pendências de PERMISSÃO **Yn** sem
    /// resposta há ≥ [`AUTO_DENY_AFTER_MS`] em `now_ms`. PURO (relógio injetado; sem
    /// timer/thread). NÃO escreve no PTY nem apenda eventos (fronteira F1-1-8: o write
    /// da recusa, com check de tela, é do executor) — devolve apenas a IDENTIFICAÇÃO.
    ///
    /// O chamador drena cada candidato pelo MESMO pipeline validado do executor com
    /// `decision=Deny, via=Timeout` (simetria total do §3: tela válida ⇒ escreve a
    /// recusa; tela divergiu ⇒ aborta sem escrever, deny-não-entregue). Como o candidato
    /// **não carrega decisão**, ZERO caminho daqui produz approve (sem knob de
    /// auto-approve — anti-regressão AC-0021.5).
    ///
    /// Recortes: **custódia** fica de fora (gate próprio da pump — ⌘⏎); **não-`Yn`**
    /// (`Choice`/`Trust`) fica de fora (alerta+foco; digitar `n` numa caixa de escolha
    /// seria input errado — mesma regra de [`AttentionQueue::resolve`]).
    #[must_use]
    pub fn auto_deny_due(&self, now_ms: u64) -> Vec<AutoDenyDue> {
        self.permissions
            .iter()
            .filter(|p| p.prompt_kind == PromptKind::Yn)
            .filter(|p| now_ms.saturating_sub(p.created_ts) >= AUTO_DENY_AFTER_MS)
            .map(|p| AutoDenyDue {
                stable_id: p.stable_id.clone(),
                node_id: p.node_id.clone(),
                vt_snapshot_hash: p.vt_snapshot_hash.clone(),
            })
            .collect()
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

    // ───────────── §3 · driver de auto-deny aos 10 min (NUNCA approve) ─────────────

    /// SLA de auto-deny (ADR 0021 §3): pendência de PERMISSÃO **Yn** sem resposta há ≥
    /// [`AUTO_DENY_AFTER_MS`] é candidata a NEGAÇÃO automática — PURO de `now_ms` (sem
    /// timer/thread). NÃO-Yn (`Choice`/`Trust`) fica de fora (alerta+foco; injetar tecla
    /// seria input errado); custódia fica de fora (gate próprio da pump). O threshold é
    /// DISTINTO e MAIOR que o escalate (5 min). O candidato NÃO carrega decisão — quem
    /// drena fixa Deny/Timeout pelo executor; por construção a fila nunca produz approve.
    #[test]
    fn auto_deny_due_after_ten_minutes_yn_only() {
        // A ordem escalate (5 min) < auto-deny (10 min) é invariante de COMPILAÇÃO
        // (`ESCALATE_BEFORE_AUTO_DENY`, no nível do módulo); aqui pino só o valor.
        assert_eq!(AUTO_DENY_AFTER_MS, 600_000, "10 min (calibração ADR 0020)");

        let mut q = AttentionQueue::new();
        // Yn com Captura 1 (o que o executor usará como `expected_hash` no check de tela).
        q.observe(
            &ask_kind(
                "A",
                PermissionEvidence::Grid,
                "yn1",
                PromptKind::Yn,
                Some("cap-yn"),
            ),
            T0,
        );
        // Choice: NUNCA auto-denied pela fila (defesa em profundidade além do gate de UI).
        q.observe(
            &ask_kind(
                "B",
                PermissionEvidence::Grid,
                "ch1",
                PromptKind::Choice,
                Some("cap-ch"),
            ),
            T0,
        );
        // Custódia: gate próprio da pump (⌘⏎) — fora do auto-deny.
        q.custody_enqueued("cust1", "C", "lina do deploy", T0);

        // Antes dos 10 min: nada vence (CONTROLE: os 3 itens existem e seguem pendentes).
        assert!(
            q.auto_deny_due(T0 + AUTO_DENY_AFTER_MS - 1).is_empty(),
            "antes de 10 min: nada vence"
        );
        assert_eq!(
            q.items(T0 + AUTO_DENY_AFTER_MS - 1).len(),
            3,
            "controle positivo: os 3 itens seguem pendentes"
        );

        // Aos 10 min: SÓ o Yn vence; carrega o binding (nó) + a Captura 1.
        let due = q.auto_deny_due(T0 + AUTO_DENY_AFTER_MS);
        assert_eq!(
            due.len(),
            1,
            "só o Yn vence (Choice e custódia ficam de fora)"
        );
        assert_eq!(due[0].stable_id, "yn1");
        assert_eq!(
            due[0].node_id, "A",
            "binding de fonte interna (PermissionAsked), nunca posição da fila"
        );
        assert_eq!(
            due[0].vt_snapshot_hash.as_deref(),
            Some("cap-yn"),
            "Captura 1 vira o expected_hash do check de tela do executor"
        );
    }

    /// Escada do SLA no MESMO item Yn: Pending → Escalated (visual, 5 min) → candidato a
    /// auto-deny (10 min). O escalate é estado VISUAL; o auto-deny é IDENTIFICAÇÃO p/ o
    /// executor — nenhum write/decisão nasce na fila (fronteira F1-1-8). Item hook-only
    /// (sem Captura 1) também vence: o executor aborta fail-safe (deny-não-entregue).
    #[test]
    fn sla_ladder_escalate_then_auto_deny() {
        let mut q = AttentionQueue::new();
        q.observe(&ask("A", PermissionEvidence::Hook, "p1"), T0); // hook-only: sem Captura 1

        // 5 min: visual Escalated, mas AINDA não vence o auto-deny.
        assert_eq!(
            q.items(T0 + ESCALATE_AFTER_MS)[0].state,
            AttentionState::Escalated
        );
        assert!(
            q.auto_deny_due(T0 + ESCALATE_AFTER_MS).is_empty(),
            "5 min escala (visual), não nega"
        );

        // 10 min: continua Escalated E agora vence o auto-deny (Captura 1 ausente → None).
        assert_eq!(
            q.items(T0 + AUTO_DENY_AFTER_MS)[0].state,
            AttentionState::Escalated
        );
        let due = q.auto_deny_due(T0 + AUTO_DENY_AFTER_MS);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].vt_snapshot_hash, None, "hook-only não tem Captura 1");
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

    /// INVARIANTE do fix P0 (anti-travamento): aplicar SÓ os eventos novos (`seq > cursor`) a uma
    /// fila viva, em dois lotes, produz EXATAMENTE o mesmo estado que reconstruir do log inteiro
    /// (`replay`). É a equivalência que autoriza tirar o full-replay `O(N)` da thread de UI.
    #[test]
    #[serial]
    fn incremental_observe_records_equals_full_replay() {
        let tmp = TempDir::new("incremental");
        let mut store = EventStore::open(tmp.path()).expect("open");
        for ev in [
            ask("A", PermissionEvidence::Hook, "p1"),
            ask("B", PermissionEvidence::Grid, "p2"),
            ask("C", PermissionEvidence::Hook, "p3"),
            DomainEvent::PermissionResolved {
                stable_id: "p1".into(),
                decision: ApprovalDecision::Approve,
                via: ResolutionVia::Human,
            },
            ask("D", PermissionEvidence::Hook, "p4"),
            DomainEvent::PermissionDismissed {
                stable_id: "p3".into(),
            },
        ] {
            store.append(&ev).expect("append");
        }
        let all = store.events().expect("events");
        let now = all.last().map(|r| r.ts).unwrap_or(T0);

        // INCREMENTAL: fila viva recebe o lote [seq<=3] e depois SÓ os novos (seq>3) — cursor.
        let split = 3usize;
        let mut live = AttentionQueue::new();
        live.observe_records(&all[..split]);
        let cursor = all[..split].last().map(|r| r.seq).unwrap_or(0);
        let new_only: Vec<EventRecord> = all.iter().filter(|r| r.seq > cursor).cloned().collect();
        assert_eq!(new_only.len(), all.len() - split, "filtro seq>cursor pega só os novos");
        live.observe_records(&new_only);

        // FULL-REPLAY: reconstrói do log inteiro.
        let full = AttentionQueue::replay(&all);

        assert_eq!(live.items(now), full.items(now), "incremental ≡ full-replay");
    }

    /// **SEAM-1: banner de spawn cascata.** `SpawnRequested`+`SpawnGated{cascade}` → item `Spawn`
    /// pendente; ORIGEM (sem `SpawnGated`) NÃO vira banner; `decline_spawn` devolve `SpawnDeclined`
    /// e, observado, tira o banner.
    #[test]
    fn spawn_cascade_becomes_pending_banner_and_declines() {
        let by = uuid::Uuid::from_u128(1);
        let mut q = AttentionQueue::new();

        // ORIGEM: SpawnRequested SEM SpawnGated → NÃO é banner (auto-admite, sem gate humano).
        q.observe(
            &DomainEvent::SpawnRequested {
                id: "msg_orig".into(),
                requested_by: by,
                name: "@A".into(),
                role: "qa".into(),
                root_cause_id: "msg_orig".into(),
                hops: 0,
                prompt: "x".into(),
                model: None,
                effort: None,
                goal_id: None,
            },
            T0,
        );
        assert!(
            q.items(T0).iter().all(|i| i.kind != AttentionKind::Spawn),
            "origem (sem SpawnGated) não vira banner"
        );

        // CASCATA: SpawnRequested + SpawnGated{cascade} → 1 banner pendente.
        q.observe(
            &DomainEvent::SpawnRequested {
                id: "msg_casc".into(),
                requested_by: by,
                name: "@Helper".into(),
                role: "helper".into(),
                root_cause_id: "R".into(),
                hops: 1,
                prompt: "ajude".into(),
                model: None,
                effort: None,
                goal_id: None,
            },
            T0 + 1,
        );
        q.observe(
            &DomainEvent::SpawnGated {
                id: "msg_casc".into(),
                requested_by: by,
                reason: "cascade".into(),
            },
            T0 + 2,
        );
        let spawn: Vec<_> = q
            .items(T0 + 3)
            .into_iter()
            .filter(|i| i.kind == AttentionKind::Spawn)
            .collect();
        assert_eq!(spawn.len(), 1, "cascata → 1 banner pendente");
        assert_eq!(spawn[0].stable_id, "msg_casc");
        assert!(
            spawn[0]
                .detail
                .as_deref()
                .unwrap_or_default()
                .contains("helper"),
            "copy leiga traz o papel pedido"
        );
        assert!(q.is_pending_spawn("msg_casc"));

        // RECUSA: decline_spawn → SpawnDeclined; observado, sai do banner.
        let decl = q.decline_spawn("msg_casc").expect("declinável");
        assert!(matches!(&decl, DomainEvent::SpawnDeclined { id } if id == "msg_casc"));
        q.observe(&decl, T0 + 4);
        assert!(!q.is_pending_spawn("msg_casc"), "recusado → sai do banner");
    }

    /// Aprovação: `SpawnAdmitted` (apendado pelo app no admit) tira o banner (resolução + dedupe M3).
    #[test]
    fn spawn_admitted_clears_banner() {
        let by = uuid::Uuid::from_u128(1);
        let node = uuid::Uuid::from_u128(2);
        let mut q = AttentionQueue::new();
        q.observe(
            &DomainEvent::SpawnRequested {
                id: "m".into(),
                requested_by: by,
                name: "@H".into(),
                role: "h".into(),
                root_cause_id: "R".into(),
                hops: 1,
                prompt: "p".into(),
                model: None,
                effort: None,
                goal_id: None,
            },
            T0,
        );
        q.observe(
            &DomainEvent::SpawnGated {
                id: "m".into(),
                requested_by: by,
                reason: "cascade".into(),
            },
            T0 + 1,
        );
        assert!(q.is_pending_spawn("m"));
        q.observe(
            &DomainEvent::SpawnAdmitted {
                id: "m".into(),
                node,
            },
            T0 + 2,
        );
        assert!(
            !q.is_pending_spawn("m"),
            "admitido → sai do banner (resolvido)"
        );
    }

    // ───────────────────────── FIX-2: ask do guard (hook PreToolUse) ─────────────────────────

    /// `ActionGated{decision:"ask", node:Some(nó)}` — o ask do guard que bloqueia o agente.
    fn guard_ask(node: &str, cmd: &str) -> DomainEvent {
        DomainEvent::ActionGated {
            cmd: cmd.into(),
            class: "gated-hard".into(),
            decision: "ask".into(),
            node: Some(node.into()),
        }
    }

    /// **FIX-2: o ASK do guard vira item `GuardAsk` (alerta + foco).** Carrega o NÓ (foco POR NOME)
    /// e o COMANDO (copy leiga "vá ao terminal aprovar X"). É o buraco do dogfooding: o guard
    /// bloqueia mas o detector F1-1-6 não pega o formato do dialog de hook-ask.
    #[test]
    fn guard_ask_becomes_guard_ask_item() {
        let mut q = AttentionQueue::new();
        q.observe(&guard_ask("Bug Finder", "git push --force origin main"), T0);
        let items = q.items(T0 + 1_000);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, AttentionKind::GuardAsk);
        assert_eq!(items[0].node_id, "Bug Finder", "nó p/ foco por nome");
        assert_eq!(
            items[0].detail.as_deref(),
            Some("git push --force origin main")
        );
    }

    /// **FIX-2: dedup por nó (N→1).** O guard dispara 1×/tool gated; o MESMO nó nunca empilha — 1
    /// item, e o último ask renova o comando exibido.
    #[test]
    fn guard_ask_dedups_per_node_keeping_latest_cmd() {
        let mut q = AttentionQueue::new();
        q.observe(&guard_ask("A", "rm -rf build"), T0);
        q.observe(&guard_ask("A", "git push --force"), T0 + 5_000);
        q.observe(&guard_ask("A", "deploy prod"), T0 + 9_000);
        let items: Vec<_> = q
            .items(T0 + 10_000)
            .into_iter()
            .filter(|i| i.kind == AttentionKind::GuardAsk)
            .collect();
        assert_eq!(items.len(), 1, "3 asks do mesmo nó → 1 item");
        assert_eq!(
            items[0].detail.as_deref(),
            Some("deploy prod"),
            "último comando renova a copy"
        );
    }

    /// **FIX-2: só `ask` COM nó alerta.** `deny`/`allow` (não bloqueiam o humano) e `ask` SEM nó
    /// (custódia/log antigo — sem terminal p/ focar) NUNCA viram item.
    #[test]
    fn deny_allow_and_node_less_ask_never_alert() {
        let mut q = AttentionQueue::new();
        let mk = |decision: &str, node: Option<&str>| DomainEvent::ActionGated {
            cmd: "x".into(),
            class: "gated-hard".into(),
            decision: decision.into(),
            node: node.map(str::to_string),
        };
        q.observe(&mk("deny", Some("A")), T0);
        q.observe(&mk("allow", Some("A")), T0 + 1);
        q.observe(&mk("ask", None), T0 + 2); // custódia / log antigo
        assert!(
            q.items(T0 + 1_000)
                .iter()
                .all(|i| i.kind != AttentionKind::GuardAsk),
            "nenhum desses vira GuardAsk"
        );
    }

    /// **FIX-2: resolução v1 por TTL.** O item some `GUARD_ASK_TTL_MS` após o ÚLTIMO ask (no core
    /// não há sinal de resolução determinístico — `recolhe-no-Busy` é follow-up de app). Um novo ask
    /// antes de expirar RENOVA a janela.
    #[test]
    fn guard_ask_expires_by_ttl_and_renews() {
        let mut q = AttentionQueue::new();
        q.observe(&guard_ask("A", "deploy"), T0);
        assert_eq!(
            q.items(T0 + GUARD_ASK_TTL_MS - 1).len(),
            1,
            "vivo dentro do TTL"
        );
        assert_eq!(q.items(T0 + GUARD_ASK_TTL_MS).len(), 0, "expira no TTL");
        // Novo ask 1ms antes de expirar → renova a janela (dedup + renovação do last_ts).
        q.observe(&guard_ask("A", "deploy"), T0 + GUARD_ASK_TTL_MS - 1);
        assert_eq!(
            q.items(T0 + GUARD_ASK_TTL_MS + 100).len(),
            1,
            "novo ask renova o TTL"
        );
    }

    /// **FIX-2: precedência custódia > permissão > spawn > guard-ask.** O ask do guard fica ABAIXO de
    /// tudo (um turno travado por permissão/custódia/spawn decide primeiro), mas NUNCA invisível.
    #[test]
    fn guard_ask_precedence_is_last() {
        let by = uuid::Uuid::from_u128(1);
        let mut q = AttentionQueue::new();
        q.observe(&guard_ask("G", "deploy"), T0);
        q.observe(&ask("P", PermissionEvidence::Hook, "p1"), T0 + 1);
        q.custody_enqueued("c1", "C", "lina do deploy", T0 + 2);
        q.observe(
            &DomainEvent::SpawnRequested {
                id: "s1".into(),
                requested_by: by,
                name: "@H".into(),
                role: "h".into(),
                root_cause_id: "R".into(),
                hops: 1,
                prompt: "p".into(),
                model: None,
                effort: None,
                goal_id: None,
            },
            T0 + 3,
        );
        q.observe(
            &DomainEvent::SpawnGated {
                id: "s1".into(),
                requested_by: by,
                reason: "cascade".into(),
            },
            T0 + 4,
        );
        let kinds: Vec<AttentionKind> = q.items(T0 + 5).into_iter().map(|i| i.kind).collect();
        assert_eq!(
            kinds,
            vec![
                AttentionKind::Custody,
                AttentionKind::Permission,
                AttentionKind::Spawn,
                AttentionKind::GuardAsk,
            ]
        );
    }

    /// **FIX-2: crash + reabrir reconstrói o GuardAsk (replay ≡ live).** O `ActionGated{ask}` está no
    /// log → reabrir a fila re-mostra o alerta (durabilidade — invariantes #4/#6).
    #[test]
    #[serial]
    fn guard_ask_survives_replay() {
        let tmp = TempDir::new("guardask");
        let mut store = EventStore::open(tmp.path()).expect("open");
        store
            .append(&guard_ask("Bug Finder", "deploy prod"))
            .expect("append");
        let records = store.events().expect("events");
        let now = records.last().map(|r| r.ts).unwrap_or(T0) + 1_000;
        let rebuilt = AttentionQueue::replay(&records);
        let items: Vec<_> = rebuilt
            .items(now)
            .into_iter()
            .filter(|i| i.kind == AttentionKind::GuardAsk)
            .collect();
        assert_eq!(items.len(), 1, "GuardAsk reconstruído do log");
        assert_eq!(items[0].node_id, "Bug Finder");
    }
}
