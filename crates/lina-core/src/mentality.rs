//! Mentality por papel (F3-3 — spec 35 §3-§4): a peça de MEMÓRIA da personalidade da Lina.
//!
//! A projeção `Mentality(papel)` reconstrói o estado das crenças de cada PAPEL do roster por
//! REPLAY do event log (inv #4, ADR 0005 — padrão `CostLedger`/`intelligence_adoption`), e a
//! política de promoção decide DETERMINISTICAMENTE (sem LLM, inv #1 — contadores sobre o log)
//! quando uma correção do usuário vira crença estabelecida.
//!
//! Eventos-fonte (`events.rs` §3): `CorrectionObserved` → `BeliefProposed` (provisória) →
//! `BeliefReinforced{situation_hash}` (N distintos) → `BeliefEstablished` | `BeliefChallenged`
//! (zera) | `BeliefRetired{reason}` (refutada/`expired`/`retired_by_human`).
//!
//! Regras duras a materializar aqui (M-PROMO / Terminal I):
//! - Promoção SSE N situações DISTINTAS (hash) confirmaram — a MESMA situação 2× NÃO conta
//!   (anti-gaming §6.4); `challenged` zera o progresso; provisória sem reforço em TTL expira.
//! - `now_ms` INJETADO (nunca wall-clock embutido — protocolo time-based mede contra o passado).
//! - Seleção `top-K` por recência/uso para o injetor (gate c — cap é critério, não opção).
//! - Crença é DADO comportamental, JAMAIS autoridade (§6.1; família ADR 0007). Filtros
//!   ESTRUTURAIS (sem PII, sem claims negativos sobre CLIs/ambiente — Hermes 20 A.2) ficam aqui;
//!   o filtro semântico anti-poisoning é do Refletor (M-DETECTOR).
//! - Crença NUNCA é deletada: rebaixada/aposentada, reconstruível por replay.
//!
//! A projeção é PURA sobre o log: `now_ms` NÃO entra no estado armazenado (só nas consultas de
//! seleção/TTL), então `from_records` reconstrói byte-a-byte idêntico independente do relógio.

use std::collections::{BTreeMap, BTreeSet};

use crate::events::{DomainEvent, EventRecord, EventStore, StoreError};

/// Política DETERMINÍSTICA de promoção (spec 35 §4.3/§6.4) — ZERO LLM (inv #1): contadores sobre o
/// replay, como o `CostLedger`. `N` distintos promove; TTL aposenta a provisória estagnada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromotionPolicy {
    /// `N` — situações DISTINTAS (hashes) exigidas para `Provisional → Established` (default 2).
    pub distinct_situations_required: usize,
    /// TTL de uma provisória sem reforço antes de expirar (default 30d — vocabulário stale do
    /// `lina retro`). Medido contra o `now_ms` INJETADO na consulta, nunca wall-clock embutido.
    pub provisional_ttl_ms: u64,
}

/// 30 dias em ms — TTL default da provisória (spec 35 §3, mesmo vocabulário stale do `lina retro`).
const DEFAULT_TTL_MS: u64 = 30 * 86_400_000;

impl Default for PromotionPolicy {
    fn default() -> Self {
        Self {
            distinct_situations_required: 2,
            provisional_ttl_ms: DEFAULT_TTL_MS,
        }
    }
}

/// Estado de leitura de uma crença na projeção — DERIVADO por replay (spec 35 §3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BeliefStatus {
    /// Em quarentena: proposta, ainda sem N situações distintas. Injetável como hipótese marcada.
    Provisional,
    /// Política determinística cruzada (N distintos): vira REGRA do papel (injetada como regra).
    Established,
    /// Fora de circulação (refutada/`expired`/`retired_by_human`) — NUNCA injetada. Não deletada.
    Retired {
        /// Motivo carimbado no `BeliefRetired` (`refuted`/`expired`/`retired_by_human`).
        reason: String,
    },
}

/// Uma crença de papel reconstruída do log (spec 35 §3). `status` é a leitura derivada; os
/// contadores (`distinct_situations`/`reinforcements`) e a `last_activity_ms` alimentam a seleção.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Belief {
    /// Id durável (gerado na emissão, lido do log no replay).
    pub belief_id: String,
    /// Papel dono — server-side (ADR 0007), nunca do payload do agente.
    pub role: String,
    /// Statement falseável ("como pensar", Why/How-to-apply). DADO comportamental, não autoridade.
    pub statement: String,
    /// Proveniência humanizável (de qual correção/quando nasceu) — para o painel.
    pub provenance: String,
    /// §6.3: nasceu em sessão que processou conteúdo externo não-confiável (sinaliza, não bloqueia).
    pub untrusted_origin: bool,
    /// Estado derivado da política sobre os eventos (`Provisional`/`Established`/`Retired`).
    pub status: BeliefStatus,
    /// Quantas situações DISTINTAS (hashes) confirmaram — zera no `challenge` (anti-gaming §6.4).
    pub distinct_situations: usize,
    /// Total de reforços válidos observados (uso) — desempata a seleção top-K.
    pub reinforcements: u32,
    /// `ts` do último evento de ciclo de vida desta crença (recência + base do TTL).
    pub last_activity_ms: u64,
}

/// Acumulador interno do fold (mantém o conjunto de hashes para deduplicar — encapsulado, fora da
/// superfície pública da `Belief`).
struct BeliefAcc {
    role: String,
    statement: String,
    provenance: String,
    untrusted_origin: bool,
    /// Situações distintas que reforçaram (dedup por hash) — zerado no `BeliefChallenged`.
    situations: BTreeSet<String>,
    reinforcements: u32,
    last_activity_ms: u64,
    /// `Some(reason)` se um `BeliefRetired` foi visto (terminal para injeção; histórico intacto).
    retired: Option<String>,
    /// Um `BeliefEstablished` explícito apareceu no log (além da promoção por contagem).
    established_explicit: bool,
}

/// Projeção `Mentality` de TODO o roster — crenças por id + tally de correções por papel + a
/// política aplicada. Igualdade estrutural prova o replay idêntico (spec 35 §7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mentality {
    /// Crenças por `belief_id` (`BTreeMap` → ordem estável → projeção determinística).
    beliefs: BTreeMap<String, Belief>,
    /// Quantas `CorrectionObserved` por papel — contexto de auditoria/painel (§5: taxa de correção).
    corrections_observed: BTreeMap<String, u32>,
    /// Ids com um `BeliefEstablished` JÁ no log — distingue "estabelecida por contagem, marco ainda
    /// não registrado" (saída de [`Mentality::promotable`]) de "marco já gravado".
    established_explicit_ids: BTreeSet<String>,
    /// Política aplicada (N/TTL) — parte da identidade da projeção.
    policy: PromotionPolicy,
}

impl Mentality {
    /// Reconstrói a projeção do `EventStore` por replay.
    ///
    /// # Errors
    /// Falha ao ler o event log.
    pub fn replay(store: &EventStore, policy: PromotionPolicy) -> Result<Self, StoreError> {
        Ok(Self::from_records(&store.events()?, policy))
    }

    /// **Coração determinístico (PURO).** Varre os registros em ordem de log e reconstrói o estado
    /// das crenças por papel. Testável com log sintético (`ts` controlado) — não embute relógio.
    #[must_use]
    pub fn from_records(records: &[EventRecord], policy: PromotionPolicy) -> Self {
        let mut accs: BTreeMap<String, BeliefAcc> = BTreeMap::new();
        let mut corrections_observed: BTreeMap<String, u32> = BTreeMap::new();

        for record in records {
            // Registro de versão futura/indecodificável: a projeção é DERIVADA, não validadora do
            // log (espelha `ApprovalExecutor::replay`) — pula sem panicar.
            let Ok(event) =
                DomainEvent::from_record(&record.kind, record.version, record.payload.clone())
            else {
                continue;
            };
            let ts = record.ts;
            match event {
                DomainEvent::CorrectionObserved { role, .. } => {
                    *corrections_observed.entry(role).or_insert(0) += 1;
                }
                DomainEvent::BeliefProposed {
                    belief_id,
                    role,
                    statement,
                    provenance,
                    untrusted_origin,
                } => {
                    // Primeira proposta cria; uma re-proposta do mesmo id NÃO reseta o progresso
                    // (idempotente sob replay — `or_insert_with` só age na ausência).
                    accs.entry(belief_id).or_insert_with(|| BeliefAcc {
                        role,
                        statement,
                        provenance,
                        untrusted_origin,
                        situations: BTreeSet::new(),
                        reinforcements: 0,
                        last_activity_ms: ts,
                        retired: None,
                        established_explicit: false,
                    });
                }
                DomainEvent::BeliefReinforced {
                    belief_id,
                    situation_hash,
                    ..
                } => {
                    if let Some(acc) = accs.get_mut(&belief_id) {
                        if acc.retired.is_none() {
                            acc.last_activity_ms = ts;
                            // Anti-gaming (§6.4): só hash NÃO-vazio conta como situação distinta.
                            let hash = situation_hash.trim();
                            if !hash.is_empty() && acc.situations.insert(hash.to_string()) {
                                acc.reinforcements = acc.reinforcements.saturating_add(1);
                            }
                        }
                    }
                }
                DomainEvent::BeliefChallenged { belief_id, .. } => {
                    if let Some(acc) = accs.get_mut(&belief_id) {
                        if acc.retired.is_none() {
                            // Usuário corrigiu de novo: ZERA o progresso (rebaixa a estabelecida).
                            acc.situations.clear();
                            acc.last_activity_ms = ts;
                        }
                    }
                }
                DomainEvent::BeliefEstablished { belief_id } => {
                    if let Some(acc) = accs.get_mut(&belief_id) {
                        if acc.retired.is_none() {
                            acc.established_explicit = true;
                            acc.last_activity_ms = ts;
                        }
                    }
                }
                DomainEvent::BeliefRetired { belief_id, reason } => {
                    if let Some(acc) = accs.get_mut(&belief_id) {
                        // Aposenta (terminal para injeção); a crença NÃO é deletada (inv #4).
                        acc.retired = Some(reason);
                        acc.last_activity_ms = ts;
                    }
                }
                _ => {}
            }
        }

        let mut established_explicit_ids = BTreeSet::new();
        let beliefs = accs
            .into_iter()
            .map(|(belief_id, acc)| {
                if acc.established_explicit {
                    established_explicit_ids.insert(belief_id.clone());
                }
                let status = finalize_status(&acc, &policy);
                let belief = Belief {
                    belief_id: belief_id.clone(),
                    role: acc.role,
                    statement: acc.statement,
                    provenance: acc.provenance,
                    untrusted_origin: acc.untrusted_origin,
                    status,
                    distinct_situations: acc.situations.len(),
                    reinforcements: acc.reinforcements,
                    last_activity_ms: acc.last_activity_ms,
                };
                (belief_id, belief)
            })
            .collect();

        Self {
            beliefs,
            corrections_observed,
            established_explicit_ids,
            policy,
        }
    }

    /// Uma crença por id (existe mesmo aposentada — crença nunca é deletada, §inv #4).
    #[must_use]
    pub fn belief(&self, belief_id: &str) -> Option<&Belief> {
        self.beliefs.get(belief_id)
    }

    /// Todas as crenças de um papel (qualquer estado), em ordem estável por id.
    #[must_use]
    pub fn beliefs_for_role(&self, role: &str) -> Vec<&Belief> {
        self.beliefs.values().filter(|b| b.role == role).collect()
    }

    /// Quantas correções (`CorrectionObserved`) o papel acumulou (auditoria; §5 taxa de correção).
    #[must_use]
    pub fn corrections_observed(&self, role: &str) -> u32 {
        self.corrections_observed.get(role).copied().unwrap_or(0)
    }

    /// **Seleção top-K para o injetor (gate c — cap é critério, não opção).** Devolve até `k`
    /// crenças injetáveis do papel: estabelecidas primeiro, depois provisórias, por recência/uso.
    /// Exclui aposentadas, provisórias expiradas (TTL contra `now_ms` INJETADO) e não-capturáveis
    /// (PII/ruído de ambiente — nunca injeta, §6.6/Hermes A.2).
    #[must_use]
    pub fn top_k_for_role(&self, role: &str, k: usize, now_ms: u64) -> Vec<&Belief> {
        let mut selected: Vec<&Belief> = self
            .beliefs
            .values()
            .filter(|b| b.role == role)
            .filter(|b| self.is_injectable(b, now_ms))
            .collect();
        // Estabelecidas primeiro; dentro do mesmo tier, por recência e depois uso (gate c). O id
        // desempata por último → ordem TOTAL determinística (cap reprodutível).
        selected.sort_by(|a, b| {
            injection_tier(a)
                .cmp(&injection_tier(b))
                .then(b.last_activity_ms.cmp(&a.last_activity_ms))
                .then(b.reinforcements.cmp(&a.reinforcements))
                .then(a.belief_id.cmp(&b.belief_id))
        });
        selected.truncate(k);
        selected
    }

    /// Uma crença entra na injeção? Aposentada nunca; estabelecida sempre (capturável garantido em
    /// [`finalize_status`]); provisória só se capturável E dentro do TTL (contra `now_ms` injetado).
    fn is_injectable(&self, belief: &Belief, now_ms: u64) -> bool {
        match belief.status {
            BeliefStatus::Retired { .. } => false,
            BeliefStatus::Established => true,
            BeliefStatus::Provisional => {
                is_capturable(&belief.statement) && !self.provisional_expired(belief, now_ms)
            }
        }
    }

    /// Provisória estagnada além do TTL (sem reforço): `now_ms - last_activity > ttl`. `now_ms` é
    /// INJETADO (o protocolo time-based mede contra o relógio passado, nunca wall-clock embutido).
    fn provisional_expired(&self, belief: &Belief, now_ms: u64) -> bool {
        now_ms.saturating_sub(belief.last_activity_ms) > self.policy.provisional_ttl_ms
    }

    /// Provisórias estagnadas além do TTL (contra `now_ms` INJETADO) — a lista que um emissor
    /// usaria para registrar `BeliefRetired{expired}`. Esta projeção NÃO emite (read-only).
    #[must_use]
    pub fn expired_provisional(&self, now_ms: u64) -> Vec<&Belief> {
        self.beliefs
            .values()
            .filter(|b| matches!(b.status, BeliefStatus::Provisional))
            .filter(|b| self.provisional_expired(b, now_ms))
            .collect()
    }

    /// Crenças que cruzaram o limiar por CONTAGEM mas ainda não têm um `BeliefEstablished` no log —
    /// a saída da política para um emissor registrar o marco idempotentemente (M-DETECTOR). Esta
    /// projeção NÃO emite (read-only, padrão `CostLedger`).
    #[must_use]
    pub fn promotable(&self) -> Vec<&Belief> {
        self.beliefs
            .values()
            .filter(|b| matches!(b.status, BeliefStatus::Established))
            .filter(|b| !self.established_explicit_ids.contains(&b.belief_id))
            .collect()
    }
}

/// Estado derivado da política sobre um acumulador (spec 35 §3/§4.3). Aposentada vence tudo; senão
/// `Established` SSE (N distintos OU marco explícito) E o statement é capturável — o filtro
/// estrutural é PISO DURO: nem um `BeliefEstablished` explícito injeta PII/ruído (§6.6/Hermes A.2).
fn finalize_status(acc: &BeliefAcc, policy: &PromotionPolicy) -> BeliefStatus {
    if let Some(reason) = &acc.retired {
        return BeliefStatus::Retired {
            reason: reason.clone(),
        };
    }
    let count_qualifies = acc.situations.len() >= policy.distinct_situations_required;
    if (count_qualifies || acc.established_explicit) && is_capturable(&acc.statement) {
        BeliefStatus::Established
    } else {
        BeliefStatus::Provisional
    }
}

/// Prioridade de injeção: estabelecida (regra) antes de provisória (hipótese). Aposentada é filtrada
/// antes de ordenar — o braço existe só para a ordem ser TOTAL (determinismo).
fn injection_tier(belief: &Belief) -> u8 {
    match belief.status {
        BeliefStatus::Established => 0,
        BeliefStatus::Provisional => 1,
        BeliefStatus::Retired { .. } => 2,
    }
}

/// Filtro ESTRUTURAL (determinístico, sem LLM): o statement pode virar crença durável/injetável?
/// Barra PII (§6.6 LGPD) e claims negativos sobre CLIs/ambiente (Hermes 20 A.2). O filtro SEMÂNTICO
/// anti-poisoning é do Refletor (M-DETECTOR) — aqui só o que é mecânico.
#[must_use]
pub fn is_capturable(statement: &str) -> bool {
    !contains_pii(statement) && !is_environment_noise(statement)
}

/// PII estrutural (§6.6 LGPD): e-mail OU sequência longa de dígitos (telefone/CPF/CNPJ/cartão). O
/// caso semântico de nome-próprio ("o cliente João") é do Refletor — aqui só o mecânico.
fn contains_pii(statement: &str) -> bool {
    contains_email(statement) || contains_long_digit_run(statement, 11)
}

/// E-mail: um token `local@dominio.tld` com caractere de palavra no `local` e um `.` com partes
/// não-vazias no domínio (heurística estrutural, sem regex — evita unwrap de compilação).
fn contains_email(statement: &str) -> bool {
    statement.split_whitespace().any(|token| {
        let Some(at) = token.find('@') else {
            return false;
        };
        let (local, rest) = token.split_at(at);
        let domain = rest[1..].trim_matches(|c: char| !c.is_alphanumeric());
        let local_ok = local.chars().any(char::is_alphanumeric);
        let domain_ok = domain.contains('.')
            && domain
                .split('.')
                .all(|part| part.chars().any(char::is_alphanumeric));
        local_ok && domain_ok
    })
}

/// Maior corrida de dígitos ignorando separadores de telefone/CPF (` . - / ( ) +`) ≥ `min` → número
/// de pessoa. Números técnicos curtos (portas, timeouts, `ADR 0007`) ficam bem abaixo do limiar.
fn contains_long_digit_run(statement: &str, min: usize) -> bool {
    let mut run = 0usize;
    for ch in statement.chars() {
        if ch.is_ascii_digit() {
            run += 1;
            if run >= min {
                return true;
            }
        } else if !matches!(ch, ' ' | '.' | '-' | '/' | '(' | ')' | '+') {
            run = 0;
        }
    }
    false
}

/// Tokens de CLI/ferramenta/ambiente (palavra inteira — evita `cli`⊂`clientes`).
const ENV_TOOL_TOKENS: &[&str] = &[
    "cli",
    "terminal",
    "npm",
    "pnpm",
    "yarn",
    "node",
    "cargo",
    "git",
    "browser",
    "navegador",
    "claude",
    "codex",
    "gpt",
    "ferramenta",
    "ferramentas",
    "tool",
    "tools",
    "stack",
    "ambiente",
    "máquina",
    "maquina",
    "sistema",
    "sistemas",
    "setup",
    "vscode",
    "chrome",
];

/// Predicados de mau-funcionamento/depreciação. NÃO incluem a negação prescritiva pura ("não npm"),
/// para o exemplo-bandeira "use pnpm, não npm" passar — é preferência, não claim de defeito.
const DISPARAGEMENT_PHRASES: &[&str] = &[
    "não funciona",
    "nao funciona",
    "doesn't work",
    "does not work",
    "do not work",
    "don't work",
    "dont work",
    "está quebrado",
    "esta quebrado",
    "tá quebrado",
    "ta quebrado",
    "quebrado",
    "is broken",
    "broken",
    "é ruim",
    "e ruim",
    "é péssimo",
    "e pessimo",
    "é inútil",
    "e inutil",
    "is bad",
    "is useless",
    "is terrible",
    "não presta",
    "nao presta",
    "não serve",
    "nao serve",
    "sempre falha",
    "sempre trava",
    "vive travando",
    "always fails",
    "always crashes",
];

/// Claim negativo sobre CLI/ambiente (Hermes 20 A.2): ruído, não lição durável. Estrutural =
/// co-ocorrência de (palavra de tool/ambiente) E (predicado de defeito). Exige AMBOS — uma
/// preferência ("use pnpm, não npm") menciona tool mas não deprecia, então passa.
fn is_environment_noise(statement: &str) -> bool {
    let lower = statement.to_lowercase();
    let mentions_tool = lower
        .split(|c: char| !c.is_alphanumeric())
        .any(|word| !word.is_empty() && ENV_TOOL_TOKENS.contains(&word));
    let disparages = DISPARAGEMENT_PHRASES.iter().any(|p| lower.contains(p));
    mentions_tool && disparages
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Registro sintético construído do PRÓPRIO `DomainEvent` (serializado como o `append` faz —
    /// carrega a tag interna `event`), com `ts` controlado para o TTL. Exercita o decode REAL
    /// (`from_record`), não um shape mockado à mão.
    fn record(event: DomainEvent, ts: u64) -> EventRecord {
        EventRecord {
            seq: 0,
            ts,
            kind: event.kind().to_string(),
            version: event.current_version(),
            payload: serde_json::to_value(&event).expect("serializa evento de domínio"),
        }
    }

    fn proposed(belief_id: &str, role: &str, statement: &str, ts: u64) -> EventRecord {
        record(
            DomainEvent::BeliefProposed {
                belief_id: belief_id.to_string(),
                role: role.to_string(),
                statement: statement.to_string(),
                provenance: format!("correção em {ts}"),
                untrusted_origin: false,
            },
            ts,
        )
    }

    fn reinforced(belief_id: &str, situation_hash: &str, ts: u64) -> EventRecord {
        record(
            DomainEvent::BeliefReinforced {
                belief_id: belief_id.to_string(),
                situation_hash: situation_hash.to_string(),
                evidence: String::new(),
            },
            ts,
        )
    }

    fn challenged(belief_id: &str, ts: u64) -> EventRecord {
        record(
            DomainEvent::BeliefChallenged {
                belief_id: belief_id.to_string(),
                evidence: String::new(),
            },
            ts,
        )
    }

    fn correction(role: &str, summary: &str, ts: u64) -> EventRecord {
        record(
            DomainEvent::CorrectionObserved {
                role: role.to_string(),
                summary: summary.to_string(),
                refs: Vec::new(),
            },
            ts,
        )
    }

    fn d() -> PromotionPolicy {
        PromotionPolicy::default()
    }

    // ── proposta (controle +) vs reforço-órfão sem proposta (controle −) ─────────────────────────

    #[test]
    fn proposal_records_a_provisional_belief_with_provenance() {
        let m =
            Mentality::from_records(&[proposed("b1", "BACKEND", "use pnpm, não npm", 100)], d());
        let b = m.belief("b1").expect("crença proposta existe");
        assert_eq!(b.role, "BACKEND");
        assert_eq!(b.statement, "use pnpm, não npm");
        assert_eq!(b.status, BeliefStatus::Provisional);
        assert_eq!(b.distinct_situations, 0);
    }

    #[test]
    fn reinforcement_without_proposal_creates_no_belief() {
        // Reforço órfão (fora de ordem / sem a proposta-mãe) NÃO inventa crença — tolerante.
        let m = Mentality::from_records(&[reinforced("ghost", "s1", 100)], d());
        assert!(m.belief("ghost").is_none(), "reforço órfão não cria crença");
    }

    // ── promoção SÓ com situações distintas (controle + e −, anti-gaming §6.4) ────────────────────

    #[test]
    fn promotes_only_with_n_distinct_situations() {
        let m = Mentality::from_records(
            &[
                proposed("b1", "BACKEND", "use pnpm, não npm", 100),
                reinforced("b1", "situacao-A", 200),
                reinforced("b1", "situacao-B", 300),
            ],
            d(),
        );
        let b = m.belief("b1").expect("crença existe");
        assert_eq!(b.status, BeliefStatus::Established, "2 distintos promove");
        assert_eq!(b.distinct_situations, 2);
    }

    #[test]
    fn same_situation_twice_does_not_promote() {
        // O CONTROLE NEGATIVO do gate (b): mesma situação 2× é gaming — NÃO promove.
        let m = Mentality::from_records(
            &[
                proposed("b1", "BACKEND", "use pnpm, não npm", 100),
                reinforced("b1", "situacao-A", 200),
                reinforced("b1", "situacao-A", 300),
            ],
            d(),
        );
        let b = m.belief("b1").expect("crença existe");
        assert_eq!(
            b.status,
            BeliefStatus::Provisional,
            "mesma situação 2× não promove"
        );
        assert_eq!(b.distinct_situations, 1, "hash repetido conta 1");
    }

    #[test]
    fn empty_situation_hash_never_promotes() {
        // Anti-gaming: hash vazio não é situação válida — não acumula distinção.
        let m = Mentality::from_records(
            &[
                proposed("b1", "BACKEND", "x", 100),
                reinforced("b1", "", 200),
                reinforced("b1", "", 300),
            ],
            d(),
        );
        let b = m.belief("b1").expect("crença existe");
        assert_eq!(b.distinct_situations, 0);
        assert_eq!(b.status, BeliefStatus::Provisional);
    }

    // ── challenge zera o progresso (controle + estabelecida→provisória; − reacúmulo) ──────────────

    #[test]
    fn challenge_zeroes_promotion_progress() {
        let m = Mentality::from_records(
            &[
                proposed("b1", "BACKEND", "use pnpm, não npm", 100),
                reinforced("b1", "situacao-A", 200),
                reinforced("b1", "situacao-B", 300),
                challenged("b1", 400),
            ],
            d(),
        );
        let b = m.belief("b1").expect("crença existe");
        assert_eq!(
            b.status,
            BeliefStatus::Provisional,
            "challenge rebaixa a estabelecida"
        );
        assert_eq!(b.distinct_situations, 0, "challenge zera o progresso");
    }

    #[test]
    fn fresh_distinct_situations_after_challenge_repromote() {
        // Controle +: depois do challenge, novas situações distintas re-promovem.
        let m = Mentality::from_records(
            &[
                proposed("b1", "BACKEND", "x", 100),
                reinforced("b1", "situacao-A", 200),
                reinforced("b1", "situacao-B", 300),
                challenged("b1", 400),
                reinforced("b1", "situacao-C", 500),
                reinforced("b1", "situacao-D", 600),
            ],
            d(),
        );
        assert_eq!(
            m.belief("b1").expect("existe").status,
            BeliefStatus::Established
        );
    }

    // ── retired nunca injeta, mas continua reconstruível (controle + e −) ─────────────────────────

    #[test]
    fn retired_belief_is_excluded_from_injection_but_still_queryable() {
        let m = Mentality::from_records(
            &[
                proposed("b1", "BACKEND", "use pnpm, não npm", 100),
                reinforced("b1", "situacao-A", 200),
                reinforced("b1", "situacao-B", 300),
                record(
                    DomainEvent::BeliefRetired {
                        belief_id: "b1".to_string(),
                        reason: "retired_by_human".to_string(),
                    },
                    400,
                ),
            ],
            d(),
        );
        let b = m
            .belief("b1")
            .expect("aposentada NÃO é deletada — replay reconstrói");
        assert_eq!(
            b.status,
            BeliefStatus::Retired {
                reason: "retired_by_human".to_string()
            }
        );
        assert!(
            m.top_k_for_role("BACKEND", 10, 1_000).is_empty(),
            "aposentada nunca injeta"
        );
    }

    // ── cap top-K com K+1 (gate c) ───────────────────────────────────────────────────────────────

    #[test]
    fn top_k_caps_at_k_with_k_plus_one_established() {
        // 3 crenças estabelecidas do papel; K=2 → exatamente 2 injetadas.
        let mut log = Vec::new();
        for (i, id) in ["b1", "b2", "b3"].iter().enumerate() {
            let base = 100 + (i as u64) * 100;
            log.push(proposed(id, "BACKEND", &format!("regra {id}"), base));
            log.push(reinforced(id, &format!("{id}-A"), base + 10));
            log.push(reinforced(id, &format!("{id}-B"), base + 20));
        }
        let m = Mentality::from_records(&log, d());
        let top = m.top_k_for_role("BACKEND", 2, 1_000);
        assert_eq!(top.len(), 2, "K+1 estabelecidas → exatamente K");
        assert!(top.iter().all(|b| b.status == BeliefStatus::Established));
    }

    #[test]
    fn top_k_orders_established_before_provisional() {
        let m = Mentality::from_records(
            &[
                proposed("estab", "BACKEND", "regra firme", 100),
                reinforced("estab", "s-A", 110),
                reinforced("estab", "s-B", 120),
                proposed("prov", "BACKEND", "hipótese", 200),
                reinforced("prov", "s-C", 210),
            ],
            d(),
        );
        let top = m.top_k_for_role("BACKEND", 10, 1_000);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].belief_id, "estab", "estabelecida vem primeiro");
        assert_eq!(top[1].belief_id, "prov");
    }

    // ── filtro estrutural: PII + claim negativo de ambiente (controle + e −, §6.6 / Hermes A.2) ───

    #[test]
    fn flagship_preference_passes_the_structural_filter() {
        // O exemplo-bandeira da §5: "use pnpm, não npm" é PREFERÊNCIA, não claim de mau-funcionamento.
        // Tem "não" + "npm" mas DEVE passar (senão o filtro come a crença canônica).
        assert!(is_capturable("use pnpm, não npm"));
        assert!(is_capturable("clientes preferem respostas curtas"));
    }

    #[test]
    fn negative_environment_claim_is_not_capturable() {
        // Hermes 20 A.2: claim negativo sobre CLI/ambiente é ruído, não lição durável.
        assert!(!is_capturable("o npm não funciona"));
        assert!(!is_capturable("browser tools do not work"));
        assert!(!is_capturable("o stack do usuário está quebrado"));
    }

    #[test]
    fn pii_statement_is_not_capturable() {
        // §6.6 LGPD: padrões sim, pessoas não.
        assert!(!is_capturable(
            "o cliente prefere e-mail joao.silva@example.com"
        ));
        assert!(!is_capturable("ligar para 11987654321 sempre"));
        assert!(is_capturable(
            "clientes preferem ser chamados pelo primeiro nome"
        ));
    }

    #[test]
    fn noise_belief_with_n_distinct_situations_is_not_promoted() {
        // INTEGRAÇÃO do filtro com a promoção: mesmo com N distintos, ruído de ambiente NÃO vira regra.
        let m = Mentality::from_records(
            &[
                proposed("noise", "BACKEND", "o npm não funciona", 100),
                reinforced("noise", "s-A", 200),
                reinforced("noise", "s-B", 300),
            ],
            d(),
        );
        assert_eq!(
            m.belief("noise").expect("existe").status,
            BeliefStatus::Provisional,
            "ruído não promove nem com N distintos"
        );
        assert!(
            m.top_k_for_role("BACKEND", 10, 1_000).is_empty(),
            "ruído nunca injeta"
        );
    }

    // ── TTL da provisória contra now_ms INJETADO (controle + expira; − dentro do prazo) ───────────

    #[test]
    fn provisional_past_ttl_expires_in_selection() {
        let m = Mentality::from_records(&[proposed("b1", "BACKEND", "hipótese", 100)], d());
        let now_expired = 100 + DEFAULT_TTL_MS + 1;
        assert!(
            m.top_k_for_role("BACKEND", 10, now_expired).is_empty(),
            "provisória além do TTL não injeta"
        );
        assert_eq!(
            m.expired_provisional(now_expired).len(),
            1,
            "lista a provisória expirada para o emissor de retire"
        );
    }

    #[test]
    fn provisional_within_ttl_is_still_injected() {
        // Controle −: dentro do prazo, a provisória ainda aparece.
        let m = Mentality::from_records(&[proposed("b1", "BACKEND", "hipótese", 100)], d());
        let now_fresh = 100 + DEFAULT_TTL_MS - 1;
        assert_eq!(m.top_k_for_role("BACKEND", 10, now_fresh).len(), 1);
        assert!(m.expired_provisional(now_fresh).is_empty());
    }

    #[test]
    fn established_belief_never_expires() {
        // TTL é só da provisória — uma regra estabelecida não some por inatividade.
        let m = Mentality::from_records(
            &[
                proposed("b1", "BACKEND", "regra firme", 100),
                reinforced("b1", "s-A", 110),
                reinforced("b1", "s-B", 120),
            ],
            d(),
        );
        let way_later = 120 + DEFAULT_TTL_MS * 10;
        assert_eq!(m.top_k_for_role("BACKEND", 10, way_later).len(), 1);
    }

    // ── correções por papel (auditoria §5) ───────────────────────────────────────────────────────

    #[test]
    fn corrections_observed_counted_per_role() {
        let m = Mentality::from_records(
            &[
                correction("BACKEND", "use pnpm", 100),
                correction("BACKEND", "outra", 200),
                correction("FRONTEND", "x", 300),
            ],
            d(),
        );
        assert_eq!(m.corrections_observed("BACKEND"), 2);
        assert_eq!(m.corrections_observed("FRONTEND"), 1);
        assert_eq!(m.corrections_observed("QA"), 0);
    }

    // ── promotable: cruzou por contagem, sem evento explícito (seam do emissor) ───────────────────

    #[test]
    fn promotable_lists_count_established_without_explicit_event() {
        let m = Mentality::from_records(
            &[
                proposed("b1", "BACKEND", "use pnpm, não npm", 100),
                reinforced("b1", "s-A", 200),
                reinforced("b1", "s-B", 300),
            ],
            d(),
        );
        let promotable = m.promotable();
        assert_eq!(promotable.len(), 1);
        assert_eq!(promotable[0].belief_id, "b1");
    }

    #[test]
    fn explicit_established_event_is_not_re_promotable() {
        // Controle −: com o BeliefEstablished já no log, não está mais "a promover".
        let m = Mentality::from_records(
            &[
                proposed("b1", "BACKEND", "use pnpm, não npm", 100),
                reinforced("b1", "s-A", 200),
                reinforced("b1", "s-B", 300),
                record(
                    DomainEvent::BeliefEstablished {
                        belief_id: "b1".to_string(),
                    },
                    400,
                ),
            ],
            d(),
        );
        assert_eq!(
            m.belief("b1").expect("existe").status,
            BeliefStatus::Established
        );
        assert!(m.promotable().is_empty(), "já registrado → não re-promove");
    }

    // ── replay reconstrói projeção idêntica (gate f, §7) ─────────────────────────────────────────

    #[test]
    fn replay_reconstructs_identical_projection() {
        let log = vec![
            proposed("b1", "BACKEND", "use pnpm, não npm", 100),
            reinforced("b1", "s-A", 200),
            reinforced("b1", "s-B", 300),
            proposed("b2", "FRONTEND", "componentes pequenos", 150),
            reinforced("b2", "s-C", 250),
            correction("BACKEND", "x", 50),
        ];
        let a = Mentality::from_records(&log, d());
        let b = Mentality::from_records(&log, d());
        assert_eq!(a, b, "replay determinístico — projeção idêntica");
    }

    /// **Roundtrip REAL (zero-mock): append no `EventStore` → `replay`.** Prova o caminho de
    /// produção ponta-a-ponta (serialização + `from_record` + promoção), não só o `from_records`
    /// puro. `append` carimba `ts` wall-clock, então asseguramos o ESTADO semântico (promoção), não
    /// o `ts` exato — o controle de `ts`/TTL fica nos testes de log sintético.
    #[test]
    fn replay_from_event_store_promotes_over_real_log() {
        let dir = std::env::temp_dir().join(format!(
            "lina-mentality-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let mut store = EventStore::open(&dir).expect("abre store temporário");
        for event in [
            DomainEvent::BeliefProposed {
                belief_id: "b1".to_string(),
                role: "BACKEND".to_string(),
                statement: "use pnpm, não npm".to_string(),
                provenance: "correção real".to_string(),
                untrusted_origin: false,
            },
            DomainEvent::BeliefReinforced {
                belief_id: "b1".to_string(),
                situation_hash: "situacao-A".to_string(),
                evidence: String::new(),
            },
            DomainEvent::BeliefReinforced {
                belief_id: "b1".to_string(),
                situation_hash: "situacao-B".to_string(),
                evidence: String::new(),
            },
        ] {
            store.append(&event).expect("append do evento de crença");
        }

        let m = Mentality::replay(&store, d()).expect("replay do store");
        let b = m.belief("b1").expect("crença reconstruída do store");
        assert_eq!(
            b.status,
            BeliefStatus::Established,
            "2 distintos no log real promove"
        );
        assert_eq!(b.distinct_situations, 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn n_required_is_configurable() {
        // A política é parâmetro: com N=3, dois distintos ainda NÃO promovem.
        let policy = PromotionPolicy {
            distinct_situations_required: 3,
            ..PromotionPolicy::default()
        };
        let m = Mentality::from_records(
            &[
                proposed("b1", "BACKEND", "x", 100),
                reinforced("b1", "s-A", 200),
                reinforced("b1", "s-B", 300),
            ],
            policy,
        );
        assert_eq!(
            m.belief("b1").expect("existe").status,
            BeliefStatus::Provisional
        );
    }
}
