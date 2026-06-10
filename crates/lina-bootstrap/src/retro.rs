//! F1-3-7 — **Auto-aprimoramento v0**: projeções puras do `lina retro` (Curator do Hermes
//! re-derivado a event-sourcing, **ZERO LLM** — quem pensa é o agente do terminal, inv#1).
//!
//! Este módulo é o miolo DETERMINÍSTICO do verbo `lina retro`: funções **puras** sobre
//! `&[EventRecord]` + um `now_ms` **injetado** (nunca wall-clock direto — lição
//! `feature-filtra-ts-wallclock`). O bin (`src/bin/lina.rs`) é a casca fina: lê o espelho
//! `log.jsonl` (como `check`/`spawn` — sem abrir o SQLite do app, evitando lock na troca de WAL),
//! parseia com [`parse_log_records`] e renderiza. Separar a lógica aqui dá:
//! - **Determinismo provável:** mesmo `&[EventRecord]` + mesmo `now_ms` ⇒ relatório
//!   byte-idêntico (critério 1). `BTreeMap`/sorts explícitos ⇒ sem não-determinismo de hash.
//! - **Replay (inv#4):** o relatório é uma **projeção** sem estado persistido — recomputado do
//!   zero a cada chamada. "Apagar o derivado e reconstruir do log" é a operação trivial de
//!   chamar `project_retro` de novo (critério 2).
//! - **Gate inviolável (critério 4):** [`classify_retro_args`] é a porta única; mutação
//!   (`archive`/`apply`/`pin`/…) retorna [`RetroInvocation::Refused`] ANTES de qualquer caminho
//!   de escrita. O verbo **só observa e sugere** — não existe `lina retro apply`.
//!
//! As SUGESTÕES (stale/archive/consolidar) são exatamente a máquina de estados do Curator
//! (`curator.py` 30d/90d, `pinned`/`absorbed_into`), mas como **texto para o humano decidir** —
//! nunca ação. O agente lê o relatório (skill `lina-retro`) e propõe; o humano aprova.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use lina_core::EventRecord;
use serde::Serialize;

/// 1 dia em millis — base de "última atividade há Xd" e dos limiares 30d/90d.
const DAY_MS: u64 = 86_400_000;

/// Limiar de **stale** (sem uso): 30 dias em millis (Curator `curator.py:58`). `> STALE_MS`.
pub const STALE_MS: u64 = 30 * 86_400_000;
/// Limiar de **archive** (candidata a arquivar): 90 dias em millis (`curator.py:59`). `> ARCHIVE_MS`.
pub const ARCHIVE_MS: u64 = 90 * 86_400_000;

/// Estado de lifecycle de uma skill — a máquina de estados do Curator como **sugestão**.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SkillState {
    /// Uso recente (`age <= 30d`).
    Active,
    /// Sem uso há mais de 30d (`> STALE_MS`) — candidata a revisão.
    Stale,
    /// Sem uso há mais de 90d (`> ARCHIVE_MS`) — candidata a arquivar (reversível).
    Archive,
}

/// Uso projetado de uma skill (de `SkillInvoked`/`SkillCreated` + `EventRecord.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillUsage {
    pub name: String,
    pub invocations: u64,
    pub creations: u64,
    /// `ts` do evento mais recente da skill (millis) — "última atividade".
    pub last_activity_ms: u64,
    /// `now_ms - last_activity_ms` (saturating) — idade desde o último uso.
    pub age_ms: u64,
    pub state: SkillState,
}

/// Cluster de nomes próximos (mesmo prefixo) — candidato a consolidação (`_render_candidate_list`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NameCluster {
    pub key: String,
    /// Skills do cluster, ordenadas.
    pub members: Vec<String>,
}

/// Gasto projetado de um nó (de `TokenUsageReported`), com papel correlacionado e flag de outlier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NodeCost {
    pub node: String,
    pub role: Option<String>,
    pub tokens: u64,
    /// `true` se `tokens > 2 × média` (e houver ≥2 nós com gasto).
    pub outlier: bool,
}

/// Contagem genérica `chave → count` (motivo de bloqueio, intenção, root, papel ausente…).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Tally {
    pub key: String,
    pub count: u64,
}

/// Gargalos de coordenação projetados do log.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Coordination {
    /// `RouteBlocked` por motivo (`no_target`, `hop_limit`, `fanout_gated`, …).
    pub blocked_by_reason: Vec<Tally>,
    /// `SpawnGated` por motivo (`cascade`, `over_cap`, `manual`, `cost`).
    pub spawn_gated_by_reason: Vec<Tally>,
    /// Re-delegações da mesma tarefa: `root_cause_id` com >1 roteamento.
    pub redelegations: Vec<Tally>,
    /// Breaker disparado: `CircuitOpened` por nó.
    pub breaker_trips: Vec<Tally>,
}

/// Relatório estruturado do `lina retro` — projeção pura do event log.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RetroReport {
    pub now_ms: u64,
    pub total_events: usize,
    /// `true` se há ≥1 `SkillInvoked`/`SkillCreated` — gate do first-run honesto (critério 5).
    pub has_skill_data: bool,
    /// Uso por skill, ordenado por nome.
    pub skills: Vec<SkillUsage>,
    /// Clusters de nomes próximos (consolidação).
    pub clusters: Vec<NameCluster>,
    pub coordination: Coordination,
    /// Gasto por nó, ordenado por nó.
    pub costs: Vec<NodeCost>,
    /// Média de tokens entre nós com gasto (base do outlier).
    pub avg_tokens: u64,
    /// O que o usuário mais pede: `MessageRouted{hops==0}` por intenção.
    pub requests_by_intent: Vec<Tally>,
    /// Lacunas de papel: `RouteBlocked{no_target}` por alvo pedido e ausente.
    pub role_gaps: Vec<Tally>,
}

/// Resultado de classificar os argumentos de `lina retro` — a porta única do gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetroInvocation {
    /// Invocação válida de RELATÓRIO (só leitura). `now_ms` sobrescreve o wall-clock (testes/demo).
    Report { json: bool, now_ms: Option<u64> },
    /// Recusada: `offending` é o token; `mutation=true` se for tentativa de aplicar mudança.
    Refused { offending: String, mutation: bool },
}

// ───────────────────────────── projeção pura ─────────────────────────────

/// Lê um campo string do payload de um registro (`None` se ausente/não-string).
fn field<'a>(rec: &'a EventRecord, key: &str) -> Option<&'a str> {
    rec.payload.get(key).and_then(serde_json::Value::as_str)
}

/// Desserializa o espelho `log.jsonl` (cada linha = um [`EventRecord`]) em registros.
/// **PURO** (sem I/O): o bin lê o arquivo e passa o conteúdo. Linhas vazias/ilegíveis são
/// IGNORADAS (mesma quarentena tolerante de `scan_spawn_outcome` — o arquivo pode estar sob append
/// concorrente do app; uma linha parcial nunca derruba o relatório, mas também nunca é inventada).
#[must_use]
pub fn parse_log_records(content: &str) -> Vec<EventRecord> {
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<EventRecord>(l).ok())
        .collect()
}

/// Projeta o relatório do `lina retro` a partir do log (`records`) e do `now_ms` injetado.
/// **PURO**: sem I/O, sem wall-clock — `now_ms` é a única fonte de "agora".
#[must_use]
pub fn project_retro(records: &[EventRecord], now_ms: u64) -> RetroReport {
    // (invocações, criações, ts da última atividade) por skill — BTreeMap ⇒ ordem estável.
    let mut skills_acc: BTreeMap<String, (u64, u64, u64)> = BTreeMap::new();
    let mut blocked: BTreeMap<String, u64> = BTreeMap::new();
    let mut role_gaps: BTreeMap<String, u64> = BTreeMap::new();
    let mut spawn_gated: BTreeMap<String, u64> = BTreeMap::new();
    let mut breaker: BTreeMap<String, u64> = BTreeMap::new();
    let mut routes_per_root: BTreeMap<String, u64> = BTreeMap::new();
    let mut intents_origin: BTreeMap<String, u64> = BTreeMap::new();
    let mut tokens_by_node: BTreeMap<String, u64> = BTreeMap::new();
    let mut role_of_node: BTreeMap<String, String> = BTreeMap::new();

    for r in records {
        match r.kind.as_str() {
            "SkillInvoked" => {
                if let Some(skill) = field(r, "skill") {
                    let e = skills_acc.entry(skill.to_string()).or_insert((0, 0, 0));
                    e.0 += 1;
                    e.2 = e.2.max(r.ts);
                }
            }
            "SkillCreated" => {
                if let Some(skill) = field(r, "skill") {
                    let e = skills_acc.entry(skill.to_string()).or_insert((0, 0, 0));
                    e.1 += 1;
                    e.2 = e.2.max(r.ts);
                }
            }
            "RouteBlocked" => {
                if let Some(reason) = field(r, "reason") {
                    *blocked.entry(reason.to_string()).or_default() += 1;
                    // Lacuna de papel: `no_target` = ninguém tem o papel/alvo pedido (`to`).
                    if reason == "no_target" {
                        if let Some(to) = field(r, "to") {
                            *role_gaps.entry(to.to_string()).or_default() += 1;
                        }
                    }
                }
            }
            "SpawnGated" => {
                if let Some(reason) = field(r, "reason") {
                    *spawn_gated.entry(reason.to_string()).or_default() += 1;
                }
            }
            "CircuitOpened" => {
                if let Some(node) = field(r, "node") {
                    *breaker.entry(node.to_string()).or_default() += 1;
                }
            }
            "MessageRouted" => {
                if let Some(root) = field(r, "root_cause_id") {
                    *routes_per_root.entry(root.to_string()).or_default() += 1;
                }
                // Pedidos de origem: só `hops==0` (o que o humano pediu; cascata não é pedido).
                let hops = r.payload.get("hops").and_then(serde_json::Value::as_u64);
                if hops == Some(0) {
                    if let Some(intent) = field(r, "intent") {
                        *intents_origin.entry(intent.to_string()).or_default() += 1;
                    }
                }
            }
            "TokenUsageReported" => {
                if let Some(node) = field(r, "node") {
                    let tokens = r.payload.get("tokens").and_then(serde_json::Value::as_u64);
                    *tokens_by_node.entry(node.to_string()).or_default() += tokens.unwrap_or(0);
                }
            }
            "NodeRoleAssigned" => {
                if let (Some(node), Some(role)) = (field(r, "node"), field(r, "role")) {
                    role_of_node.insert(node.to_string(), role.to_string()); // último vence
                }
            }
            _ => {}
        }
    }

    let has_skill_data = !skills_acc.is_empty();

    // Skills + estado de lifecycle (sugestão; nunca ação).
    let skills: Vec<SkillUsage> = skills_acc
        .iter()
        .map(|(name, &(inv, cre, last))| {
            let age_ms = now_ms.saturating_sub(last);
            let state = if age_ms > ARCHIVE_MS {
                SkillState::Archive
            } else if age_ms > STALE_MS {
                SkillState::Stale
            } else {
                SkillState::Active
            };
            SkillUsage {
                name: name.clone(),
                invocations: inv,
                creations: cre,
                last_activity_ms: last,
                age_ms,
                state,
            }
        })
        .collect();

    // Clusters de nomes próximos (consolidação): mesmo prefixo, ≥2 membros.
    let mut clusters_acc: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for s in &skills {
        let key = cluster_key(&s.name);
        if !key.is_empty() {
            clusters_acc
                .entry(key.to_string())
                .or_default()
                .push(s.name.clone());
        }
    }
    let clusters: Vec<NameCluster> = clusters_acc
        .into_iter()
        .filter(|(_, m)| m.len() >= 2)
        .map(|(key, mut members)| {
            members.sort();
            NameCluster { key, members }
        })
        .collect();

    // Custos + outliers (> 2× a média; precisa de ≥2 nós para haver baseline).
    let total: u64 = tokens_by_node.values().sum();
    let n = tokens_by_node.len() as u64;
    let avg_tokens = total.checked_div(n).unwrap_or(0);
    let costs: Vec<NodeCost> = tokens_by_node
        .iter()
        .map(|(node, &tokens)| NodeCost {
            node: node.clone(),
            role: role_of_node.get(node).cloned(),
            tokens,
            outlier: n >= 2 && tokens > avg_tokens.saturating_mul(2),
        })
        .collect();

    // Re-delegações: só roots com >1 roteamento (a mesma tarefa rodou de novo).
    let redel: BTreeMap<String, u64> = routes_per_root
        .into_iter()
        .filter(|(_, c)| *c > 1)
        .collect();

    RetroReport {
        now_ms,
        total_events: records.len(),
        has_skill_data,
        skills,
        clusters,
        coordination: Coordination {
            blocked_by_reason: tallies_sorted(&blocked),
            spawn_gated_by_reason: tallies_sorted(&spawn_gated),
            redelegations: tallies_sorted(&redel),
            breaker_trips: tallies_sorted(&breaker),
        },
        costs,
        avg_tokens,
        requests_by_intent: tallies_sorted(&intents_origin),
        role_gaps: tallies_sorted(&role_gaps),
    }
}

/// Renderiza um bloco `cabeçalho:\n  - chave: count` só quando há tallies (senão nada).
fn render_tallies(out: &mut String, header: &str, tallies: &[Tally], suffix: &str) {
    if tallies.is_empty() {
        return;
    }
    let _ = writeln!(out, "{header}");
    for t in tallies {
        let _ = writeln!(out, "  - {}: {}{}", t.key, t.count, suffix);
    }
}

/// Renderiza o relatório em texto determinístico (ASCII, estilo do bin) — o que a skill
/// `lina-retro` lê. SOMENTE descreve e SUGERE; jamais um caminho de ação.
#[must_use]
pub fn render_report(r: &RetroReport) -> String {
    let mut o = String::new();
    let _ = writeln!(
        o,
        "LINA RETRO -- relatorio do Espaco (auto-aprimoramento v0)"
    );
    let _ = writeln!(o, "gerado (epoch ms): {}", r.now_ms);
    let _ = writeln!(o, "eventos no log: {}", r.total_events);
    let _ = writeln!(
        o,
        "ESTE RELATORIO SO OBSERVA E SUGERE -- nunca aplica. Toda manutencao (skills/papeis/presets) passa por gate humano."
    );

    // ── SKILLS ──
    let _ = writeln!(o, "\n== SKILLS ==");
    if !r.has_skill_data {
        let _ = writeln!(
            o,
            "dados insuficientes: nenhuma skill foi invocada ou criada no log (first-run). Sem sugestao de manutencao."
        );
    } else {
        for s in &r.skills {
            let estado = match s.state {
                SkillState::Active => "ATIVA",
                SkillState::Stale => "STALE (>30d)",
                SkillState::Archive => "ARCHIVE (>90d)",
            };
            let _ = writeln!(
                o,
                "  - {}: {} invocacoes, {} criacoes, ult. atividade ha {}d -> {estado}",
                s.name,
                s.invocations,
                s.creations,
                s.age_ms / DAY_MS
            );
        }
        let archives: Vec<&SkillUsage> = r
            .skills
            .iter()
            .filter(|s| s.state == SkillState::Archive)
            .collect();
        let stales: Vec<&SkillUsage> = r
            .skills
            .iter()
            .filter(|s| s.state == SkillState::Stale)
            .collect();
        if !archives.is_empty() || !stales.is_empty() {
            let _ = writeln!(o, "sugestoes (gate humano -- este verbo NAO aplica):");
            for s in archives {
                let _ = writeln!(
                    o,
                    "  - arquivar '{}' (sem uso ha {}d > 90d; reversivel -- arquivar != deletar)",
                    s.name,
                    s.age_ms / DAY_MS
                );
            }
            for s in stales {
                let _ = writeln!(
                    o,
                    "  - revisar '{}' (sem uso ha {}d > 30d)",
                    s.name,
                    s.age_ms / DAY_MS
                );
            }
        }
        if !r.clusters.is_empty() {
            let _ = writeln!(o, "consolidacao (nomes proximos -- avaliar unificar):");
            for c in &r.clusters {
                let _ = writeln!(
                    o,
                    "  - {}: {} ({} skills)",
                    c.key,
                    c.members.join(", "),
                    c.members.len()
                );
            }
        }
    }

    // ── COORDENACAO ──
    let _ = writeln!(o, "\n== COORDENACAO ==");
    let co = &r.coordination;
    let coord_empty = co.blocked_by_reason.is_empty()
        && co.spawn_gated_by_reason.is_empty()
        && co.redelegations.is_empty()
        && co.breaker_trips.is_empty();
    if coord_empty {
        let _ = writeln!(o, "sem gargalos de coordenacao no log.");
    } else {
        render_tallies(
            &mut o,
            "roteamentos bloqueados por motivo:",
            &co.blocked_by_reason,
            "",
        );
        render_tallies(
            &mut o,
            "spawns barrados/adiados por motivo:",
            &co.spawn_gated_by_reason,
            "",
        );
        render_tallies(
            &mut o,
            "re-delegacoes da mesma tarefa (por root_cause_id, >1 roteamento):",
            &co.redelegations,
            " roteamentos",
        );
        render_tallies(&mut o, "breaker disparado (por no):", &co.breaker_trips, "");
    }

    // ── CUSTOS ──
    let _ = writeln!(o, "\n== CUSTOS ==");
    if r.costs.is_empty() {
        let _ = writeln!(o, "sem uso de tokens reportado.");
    } else {
        let _ = writeln!(
            o,
            "gasto por terminal (tokens reportados; media {}):",
            r.avg_tokens
        );
        for c in &r.costs {
            let role = c.role.as_deref().unwrap_or("papel nao rastreavel");
            let mark = if c.outlier {
                "  <- OUTLIER (> 2x media)"
            } else {
                ""
            };
            let _ = writeln!(o, "  - {} ({role}): {} tokens{mark}", c.node, c.tokens);
        }
    }

    // ── PEDIDOS ──
    let _ = writeln!(o, "\n== PEDIDOS DO USUARIO (turnos de origem, hops==0) ==");
    if r.requests_by_intent.is_empty() {
        let _ = writeln!(o, "sem turnos de origem registrados.");
    } else {
        render_tallies(&mut o, "por intencao:", &r.requests_by_intent, "");
    }

    // ── LACUNAS ──
    let _ = writeln!(o, "\n== LACUNAS DE PAPEL ==");
    if r.role_gaps.is_empty() {
        let _ = writeln!(o, "nenhuma lacuna de papel detectada.");
    } else {
        render_tallies(
            &mut o,
            "papeis/alvos pedidos e ausentes (delegacao sem destino):",
            &r.role_gaps,
            " pedido(s)",
        );
    }

    o
}

/// Subcomandos/flags que tentariam MUTAR — recusados pelo gate (não existe `lina retro apply`).
const MUTATION_TOKENS: &[&str] = &[
    "apply",
    "archive",
    "pin",
    "unpin",
    "prune",
    "delete",
    "remove",
    "consolidate",
    "merge",
    "write",
    "fix",
    "--apply",
    "--archive",
    "--pin",
    "--unpin",
    "--prune",
    "--delete",
    "--remove",
    "--consolidate",
    "--merge",
    "--write",
    "--fix",
];

/// Classifica os args de `lina retro` (após o verbo). **Porta única do gate humano**: o verbo só
/// aceita `--json` e `--now-ms <n>` (relatório só-leitura). Qualquer token de mutação ⇒
/// [`RetroInvocation::Refused`]`{mutation:true}` ANTES de qualquer caminho de escrita — não existe,
/// por construção, aresta de um arg-de-mutação até um `append`. Posicional desconhecido também é
/// recusado (`mutation:false`), nunca degradando para uma ação.
#[must_use]
pub fn classify_retro_args(args: &[String]) -> RetroInvocation {
    let mut json = false;
    let mut now_ms = None;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if MUTATION_TOKENS.contains(&a.to_ascii_lowercase().as_str()) {
            return RetroInvocation::Refused {
                offending: a.to_string(),
                mutation: true,
            };
        }
        match a {
            "--json" => json = true,
            "--now-ms" => match args.get(i + 1).and_then(|s| s.parse::<u64>().ok()) {
                Some(v) => {
                    now_ms = Some(v);
                    i += 1;
                }
                None => {
                    return RetroInvocation::Refused {
                        offending: "--now-ms (valor ausente ou invalido)".to_string(),
                        mutation: false,
                    }
                }
            },
            other => {
                return RetroInvocation::Refused {
                    offending: other.to_string(),
                    mutation: false,
                }
            }
        }
        i += 1;
    }
    RetroInvocation::Report { json, now_ms }
}

// ───────────────────────────── helpers internos ─────────────────────────────

/// Chave de cluster: o prefixo antes do 1º separador (`-`/`_`/`:`). `""` se não houver prefixo
/// (nome atômico não agrupa — evita cluster espúrio de skills sem hierarquia de nome).
fn cluster_key(name: &str) -> &str {
    match name.find(['-', '_', ':']) {
        Some(i) => &name[..i],
        None => "",
    }
}

/// Ordena um mapa `chave→count` em `Vec<Tally>` determinístico: **count desc, depois chave asc**
/// (o `BTreeMap` já entra ordenado por chave; o sort estável por count preserva isso no desempate).
fn tallies_sorted(map: &BTreeMap<String, u64>) -> Vec<Tally> {
    let mut v: Vec<Tally> = map
        .iter()
        .map(|(k, c)| Tally {
            key: k.clone(),
            count: *c,
        })
        .collect();
    v.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.key.cmp(&b.key)));
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// UUIDs válidos (NodeId = Uuid) — para os fixtures de nó casarem o shape real no drift-guard.
    const N1: &str = "00000000-0000-0000-0000-000000000001";
    const N2: &str = "00000000-0000-0000-0000-000000000002";
    const N3: &str = "00000000-0000-0000-0000-000000000003";

    fn rec(seq: u64, ts: u64, kind: &str, payload: serde_json::Value) -> EventRecord {
        EventRecord {
            seq,
            ts,
            kind: kind.to_string(),
            version: 1,
            payload,
        }
    }

    fn skill_invoked(seq: u64, ts: u64, node: &str, skill: &str) -> EventRecord {
        rec(
            seq,
            ts,
            "SkillInvoked",
            json!({"event":"SkillInvoked","node":node,"skill":skill}),
        )
    }
    fn skill_created(seq: u64, ts: u64, node: &str, skill: &str) -> EventRecord {
        rec(
            seq,
            ts,
            "SkillCreated",
            json!({"event":"SkillCreated","node":node,"skill":skill}),
        )
    }
    fn route_blocked(seq: u64, reason: &str, to: &str) -> EventRecord {
        rec(
            seq,
            seq * 10,
            "RouteBlocked",
            json!({"event":"RouteBlocked","id":format!("m{seq}"),"reason":reason,"from":"x","to":to}),
        )
    }
    fn spawn_gated(seq: u64, reason: &str) -> EventRecord {
        rec(
            seq,
            seq * 10,
            "SpawnGated",
            json!({"event":"SpawnGated","id":format!("m{seq}"),"requested_by":N1,"reason":reason}),
        )
    }
    fn message_routed(seq: u64, root: &str, hops: u64, intent: &str) -> EventRecord {
        rec(
            seq,
            seq * 10,
            "MessageRouted",
            json!({"event":"MessageRouted","id":format!("m{seq}"),"from":N1,"to":"t","intent":intent,"root_cause_id":root,"hops":hops}),
        )
    }
    fn token_usage(seq: u64, node: &str, tokens: u64) -> EventRecord {
        rec(
            seq,
            seq * 10,
            "TokenUsageReported",
            json!({"event":"TokenUsageReported","node":node,"tokens":tokens}),
        )
    }
    fn role_assigned(seq: u64, node: &str, role: &str) -> EventRecord {
        rec(
            seq,
            seq * 10,
            "NodeRoleAssigned",
            json!({"event":"NodeRoleAssigned","node":node,"role":role}),
        )
    }
    fn circuit_opened(seq: u64, node: &str) -> EventRecord {
        rec(
            seq,
            seq * 10,
            "CircuitOpened",
            json!({"event":"CircuitOpened","node":node,"failures":3}),
        )
    }

    const NOW: u64 = 1_000_000_000_000; // ~2001; > 90d para os fixtures terem idade positiva

    /// Fixture rico — exercita TODAS as seções (base do determinismo e do snapshot mental).
    fn rich_log() -> Vec<EventRecord> {
        vec![
            // skills
            skill_created(1, NOW - 10 * 86_400_000, N1, "growthos-sales"),
            skill_invoked(2, NOW - 5 * 86_400_000, N1, "growthos-sales"),
            skill_invoked(3, NOW - 2 * 86_400_000, N2, "growthos-sales"),
            skill_invoked(4, NOW - 40 * 86_400_000, N1, "growthos-narrative"), // stale
            skill_invoked(5, NOW - 100 * 86_400_000, N2, "old-skill"),         // archive
            // coordenação
            route_blocked(6, "no_target", "designer"),
            route_blocked(7, "no_target", "designer"),
            route_blocked(8, "hop_limit", "x"),
            spawn_gated(9, "cascade"),
            circuit_opened(10, N3),
            message_routed(11, "r1", 0, "ask"),
            message_routed(12, "r1", 1, "handoff"),
            message_routed(13, "r1", 1, "handoff"),
            message_routed(14, "r2", 0, "ask"),
            // custos
            token_usage(15, N1, 100),
            token_usage(16, N2, 100),
            token_usage(17, N3, 1000),
            role_assigned(18, N3, "dev"),
        ]
    }

    #[test]
    fn determinism_byte_identical_across_runs() {
        let recs = rich_log();
        let a = render_report(&project_retro(&recs, NOW));
        let b = render_report(&project_retro(&recs, NOW));
        assert_eq!(a, b, "relatório deve ser byte-idêntico em 2 execuções");
        assert!(!a.is_empty(), "relatório não pode ser vazio com log rico");
    }

    #[test]
    fn replay_pure_projection_reconstructs_identical() {
        // inv#4: sem estado derivado persistido — projetar de novo do MESMO log dá o MESMO relatório.
        let recs = rich_log();
        let first = project_retro(&recs, NOW);
        let reconstructed = project_retro(&recs.clone(), NOW);
        assert_eq!(first, reconstructed);
    }

    #[test]
    fn skill_counts_and_last_activity() {
        let recs = rich_log();
        let r = project_retro(&recs, NOW);
        let gs = r
            .skills
            .iter()
            .find(|s| s.name == "growthos-sales")
            .expect("growthos-sales presente");
        assert_eq!(gs.invocations, 2);
        assert_eq!(gs.creations, 1);
        assert_eq!(gs.last_activity_ms, NOW - 2 * 86_400_000); // o invoke mais recente vence
    }

    #[test]
    fn stale_archive_thresholds_at_boundary() {
        // now injetado: prova o limiar 30d/90d sem wall-clock (lição feature-filtra-ts-wallclock).
        let recs = vec![
            skill_invoked(1, NOW - STALE_MS, N1, "exatamente-30d"), // age == 30d → ATIVA
            skill_invoked(2, NOW - STALE_MS - 1, N1, "passou-30d"), // age > 30d → STALE
            skill_invoked(3, NOW - ARCHIVE_MS, N1, "exatamente-90d"), // age == 90d → STALE
            skill_invoked(4, NOW - ARCHIVE_MS - 1, N1, "passou-90d"), // age > 90d → ARCHIVE
        ];
        let r = project_retro(&recs, NOW);
        let st = |name: &str| r.skills.iter().find(|s| s.name == name).unwrap().state;
        assert_eq!(st("exatamente-30d"), SkillState::Active);
        assert_eq!(st("passou-30d"), SkillState::Stale);
        assert_eq!(st("exatamente-90d"), SkillState::Stale);
        assert_eq!(st("passou-90d"), SkillState::Archive);
    }

    #[test]
    fn first_run_honest_no_skill_data() {
        // Workspace recém-criado: nenhum evento de skill → "dados insuficientes", ZERO sugestão de prune.
        let recs = vec![
            rec(
                1,
                NOW,
                "WorkspaceCreated",
                json!({"event":"WorkspaceCreated","name":"w","focus_preset":""}),
            ),
            route_blocked(2, "hop_limit", "x"),
        ];
        let r = project_retro(&recs, NOW);
        assert!(!r.has_skill_data);
        assert!(r.skills.is_empty());
        let out = render_report(&r);
        assert!(
            out.contains("dados insuficientes"),
            "first-run deve declarar dados insuficientes"
        );
        assert!(
            !out.contains("arquivar"),
            "first-run NÃO pode sugerir arquivar (prune): {out}"
        );
    }

    #[test]
    fn name_clusters_group_shared_prefix() {
        let recs = vec![
            skill_invoked(1, NOW, N1, "growthos-sales"),
            skill_invoked(2, NOW, N1, "growthos-narrative"),
            skill_invoked(3, NOW, N1, "social-content"),
        ];
        let r = project_retro(&recs, NOW);
        assert_eq!(r.clusters.len(), 1, "só growthos-* forma cluster (>=2)");
        let c = &r.clusters[0];
        assert_eq!(c.key, "growthos");
        assert_eq!(c.members, vec!["growthos-narrative", "growthos-sales"]);
    }

    #[test]
    fn coordination_counts_blocks_gates_redelegations_breaker() {
        let r = project_retro(&rich_log(), NOW);
        let find = |v: &[Tally], k: &str| v.iter().find(|t| t.key == k).map(|t| t.count);
        assert_eq!(
            find(&r.coordination.blocked_by_reason, "no_target"),
            Some(2)
        );
        assert_eq!(
            find(&r.coordination.blocked_by_reason, "hop_limit"),
            Some(1)
        );
        assert_eq!(
            find(&r.coordination.spawn_gated_by_reason, "cascade"),
            Some(1)
        );
        // r1 teve 3 roteamentos (>1) → re-delegação; r2 teve 1 → NÃO entra.
        assert_eq!(find(&r.coordination.redelegations, "r1"), Some(3));
        assert_eq!(find(&r.coordination.redelegations, "r2"), None);
        assert_eq!(find(&r.coordination.breaker_trips, N3), Some(1));
    }

    #[test]
    fn costs_sum_role_and_outlier() {
        let r = project_retro(&rich_log(), NOW);
        assert_eq!(r.avg_tokens, 400); // (100+100+1000)/3
        let cost = |node: &str| r.costs.iter().find(|c| c.node == node).unwrap();
        assert_eq!(cost(N3).tokens, 1000);
        assert!(cost(N3).outlier, "1000 > 2×400 → outlier");
        assert_eq!(cost(N3).role.as_deref(), Some("dev"));
        assert!(!cost(N1).outlier);
        assert_eq!(cost(N1).role, None); // sem NodeRoleAssigned → degradação honesta
    }

    #[test]
    fn requests_by_intent_only_origin_turns() {
        let r = project_retro(&rich_log(), NOW);
        // hops==0: ask (m11) + ask (m14) = 2; handoff em hops>=1 NÃO conta (é cascata, não pedido).
        let ask = r.requests_by_intent.iter().find(|t| t.key == "ask");
        assert_eq!(ask.map(|t| t.count), Some(2));
        assert!(
            r.requests_by_intent.iter().all(|t| t.key != "handoff"),
            "handoff só apareceu em hops>=1 → fora dos pedidos de origem"
        );
    }

    #[test]
    fn role_gaps_from_no_target_blocks() {
        let r = project_retro(&rich_log(), NOW);
        let designer = r.role_gaps.iter().find(|t| t.key == "designer");
        assert_eq!(designer.map(|t| t.count), Some(2));
    }

    #[test]
    fn gate_refuses_mutation_attempts() {
        use RetroInvocation::*;
        let s =
            |a: &[&str]| classify_retro_args(&a.iter().map(|x| x.to_string()).collect::<Vec<_>>());
        // mutações: recusadas com flag mutation=true (gate humano inviolável).
        assert!(matches!(
            s(&["archive", "lina-x"]),
            Refused { mutation: true, .. }
        ));
        assert!(matches!(s(&["--apply"]), Refused { mutation: true, .. }));
        assert!(matches!(s(&["pin", "foo"]), Refused { mutation: true, .. }));
        assert!(matches!(s(&["prune"]), Refused { mutation: true, .. }));
        // válidas: relatório só-leitura.
        assert_eq!(
            s(&[]),
            Report {
                json: false,
                now_ms: None
            }
        );
        assert_eq!(
            s(&["--json"]),
            Report {
                json: true,
                now_ms: None
            }
        );
        assert_eq!(
            s(&["--now-ms", "123"]),
            Report {
                json: false,
                now_ms: Some(123)
            }
        );
        // posicional desconhecido: recusado (sem subcomando de mutação por construção), mutation=false.
        assert!(matches!(
            s(&["bogus"]),
            Refused {
                mutation: false,
                ..
            }
        ));
    }

    #[test]
    fn parse_log_records_reads_lines_and_skips_garbage() {
        let l1 = serde_json::to_string(&route_blocked(1, "no_target", "designer")).unwrap();
        let l2 = serde_json::to_string(&skill_invoked(2, 200, N1, "foo")).unwrap();
        let content = format!("{l1}\n\n{l2}\nlixo-nao-json {{\n");
        let recs = parse_log_records(&content);
        assert_eq!(recs.len(), 2, "2 linhas validas; vazia e lixo ignorados");
        assert_eq!(recs[0].kind, "RouteBlocked");
        assert_eq!(recs[1].kind, "SkillInvoked");
        assert_eq!(recs[1].ts, 200);
    }

    /// Drift-guard do seam (events.rs é do Dev 01): se o shape de `SkillInvoked`/`SkillCreated`
    /// mudar, este `from_value` falha e o teste pega ANTES de o relatório mentir.
    #[test]
    fn skill_event_shape_is_stable() {
        use lina_core::DomainEvent;
        let inv = json!({"event":"SkillInvoked","node":N1,"skill":"foo"});
        let cre = json!({"event":"SkillCreated","node":N1,"skill":"foo"});
        let a: DomainEvent = serde_json::from_value(inv).expect("SkillInvoked shape drift!");
        let b: DomainEvent = serde_json::from_value(cre).expect("SkillCreated shape drift!");
        assert_eq!(a.kind(), "SkillInvoked");
        assert_eq!(b.kind(), "SkillCreated");
    }
}
