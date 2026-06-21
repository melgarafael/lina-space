//! `intelligence_adoption` — **F3-CONF-3 R2: telemetria de ADOÇÃO da inteligência F3**.
//!
//! Leitor **offline** do `log.jsonl` canônico. Responde, a partir do event log, à pergunta que é
//! o **top-risco da Fase 3** (épico `39` §VI-3): *"a inteligência da F3 está sendo USADA de
//! verdade, ou foi construída e nunca acionada?"* — Goal/effort/Tradutor podem virar enfeite se
//! ninguém os aciona. Mede-se o uso REAL no log desde o dia 1, não se assume.
//!
//! **É MÉTRICA ACOMPANHADA, NUNCA GATE** (mesma doutrina do irmão [`plan_adoption`]): adoção é
//! comportamento de LLM, não-determinístico — o que se gateia são os freios/verbos/doutrina, não
//! este número. Este leitor não tem modo "falha": ele REPORTA o número cru (inclusive 0). A saída
//! diz isso explicitamente para o número nunca virar critério binário por engano. Um relatório que
//! sempre diz "tudo ótimo" é pior que vermelho.
//!
//! ## O que mede
//! - **Goal (funil):** quantas metas foram `GoalDefined`, quantas avançaram a `GoalInterpreted` /
//!   `GoalConfirmed` (o gate humano cruzado) / `GoalDecomposed`, e quantas FECHARAM (`GoalAchieved`)
//!   vs ESCALARAM (`GoalEscalated`, por `reason`). Taxa de fechamento = `achieved / (achieved +
//!   escalated)`, `None` quando nenhuma meta terminou (nunca `0/0`).
//! - **Tradutor (proveniência):** distribuição do `origin` das metas — mede a adoção do papel
//!   Tradutor da F3-2 (quantas metas vieram de `@Tradutor` vs degradadas ao `@Maestro` vs outra
//!   origem). Ver nota de design sobre o campo abaixo.
//! - **Effort (motor por papel, F3-0-4):** quantos `EffortAssigned` e a distribuição por nível
//!   (`low`/`medium`/`high`) e por `origin` (`observed`/`assigned`) — mede se o motor de effort
//!   está vivo.
//! - **Correção/Mentality:** slot previsto contando a sentinela `[LINA::CORRECTION]` (futuro F3-3 —
//!   hoje 0, o evento ainda não existe; ver o gancho marcado no agregador).
//!
//! ## Decisões de design (registradas)
//! - **Bin auto-descoberto** (`src/bin/`): zero edição em `lib.rs`/`Cargo.toml`/módulos com dono
//!   (multi-terminal). Mesmo padrão de [`plan_adoption`]/`baseline_f1_0`.
//! - **Lê o log EXISTENTE** — nenhum evento novo. Desserializa cada linha em [`EventRecord`] e lê os
//!   campos do `payload` por chave (`payload.get(...)`), o que é tolerante a versão: um campo
//!   ausente vira `None` e segue, nunca panic (regra "conte e siga" do despacho).
//! - **Agrupa Goal por `goal_id`** (cunhado pelo supervisor, único por meta) — uma meta = 1 em
//!   todo o funil, mesmo que o log tenha eventos repetidos por replay. É o "não inflar" aplicado à
//!   meta (o irmão `plan_adoption` faz o análogo com `root_cause_id` em cadeias de ask).
//! - **Proveniência do Tradutor vem de `GoalDefined.origin`**, NÃO de `GoalInterpreted`. O despacho
//!   R2 cita "`GoalInterpreted` com `origin:@Tradutor`", mas o contrato real em `events.rs` (dono:
//!   Terminal A) põe `origin: "@Maestro"|"@Tradutor"|"cron:<id>"` em `GoalDefined` — e o item de
//!   plano `F2TRAD` confirma "origin via GoalDefined.origin (ja existe)". `GoalInterpreted` não tem
//!   o campo. Seguimos o código (fonte da verdade), sinalizado no reporte — não se mede contra um
//!   campo inexistente.
//!
//! Uso: `intelligence_adoption <events-dir> [--json]` (ex.: `<ws>/.lina/events`).

use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;

use lina_core::EventRecord;

/// O relatório de adoção da inteligência F3 — números crus + taxas derivadas (`None` = sem
/// denominador, nunca `0/0`). Serializável para `--json` (consumo por máquina/teste).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct IntelligenceReport {
    /// Metas declaradas (`GoalDefined`), contadas por `goal_id` distinto.
    pub goals_defined: u64,
    /// Metas que receberam ao menos um `GoalInterpreted`.
    pub goals_interpreted: u64,
    /// Metas que cruzaram o gate humano (`GoalConfirmed`).
    pub goals_confirmed: u64,
    /// Metas decompostas em itens do plano (`GoalDecomposed`).
    pub goals_decomposed: u64,
    /// Metas que FECHARAM com qualidade (`GoalAchieved`).
    pub goals_achieved: u64,
    /// Metas ESCALADAS ao humano sem fechar (`GoalEscalated`) — pausa, não morte.
    pub goals_escalated: u64,
    /// `achieved / (achieved + escalated)` em %, 1 casa. `None` = nenhuma meta terminou ainda.
    pub goal_close_rate_pct: Option<f64>,
    /// Razões de escala (`stalled`/`turn_budget_exhausted`/…) → nº de metas. Vocabulário tolerante.
    pub escalation_reasons: BTreeMap<String, u64>,
    /// Proveniência das metas: `origin` cru de `GoalDefined` → nº de metas. Mede o papel Tradutor
    /// (`@Tradutor`) vs degradação ao Maestro (`@Maestro`) vs outra origem (`cron:…`, nome de
    /// terminal…). Cru de propósito: o número honesto mostra QUAL vocabulário a proveniência usa.
    pub goals_by_origin: BTreeMap<String, u64>,
    /// Atribuições de effort observadas/atribuídas (`EffortAssigned`).
    pub effort_assigned: u64,
    /// Distribuição de effort por nível (`low`/`medium`/`high`).
    pub effort_by_level: BTreeMap<String, u64>,
    /// Distribuição de effort por origem do carimbo (`observed`/`assigned`).
    pub effort_by_origin: BTreeMap<String, u64>,
    /// Correções da Mentality (`[LINA::CORRECTION]`) — **slot futuro F3-3**, hoje sempre 0 (o evento
    /// ainda não existe). Reportado explícito para o gate enxergar o slot, não um silêncio.
    pub corrections: u64,
}

/// Acumulador por meta — uma entrada por `goal_id`, para o funil nunca inflar com eventos
/// repetidos. Cada marco é um booleano "esta meta alcançou X" (idempotente sob replay).
#[derive(Default)]
struct GoalAgg {
    defined: bool,
    interpreted: bool,
    confirmed: bool,
    decomposed: bool,
    achieved: bool,
    escalated: bool,
    /// `origin` carimbado no `GoalDefined` (proveniência; ver doc do módulo).
    origin: Option<String>,
    /// `reason` carimbado no `GoalEscalated` (por que pausou).
    escalate_reason: Option<String>,
}

/// Lê `payload.<key>` como `&str`, vazio/ausente → `None`. Tolerante a versão (chave nova/velha).
fn str_field<'a>(rec: &'a EventRecord, key: &str) -> Option<&'a str> {
    rec.payload
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.trim().is_empty())
}

/// Computa o relatório sobre os registros do log — **pura** (testável com log sintético).
#[must_use]
pub fn intelligence_report(records: &[EventRecord]) -> IntelligenceReport {
    // Funil de Goal, indexado por goal_id (1 meta = 1 entrada).
    let mut goals: BTreeMap<String, GoalAgg> = BTreeMap::new();
    // Effort, contado por evento (cada EffortAssigned é uma atribuição).
    let mut effort_assigned = 0u64;
    let mut effort_by_level: BTreeMap<String, u64> = BTreeMap::new();
    let mut effort_by_origin: BTreeMap<String, u64> = BTreeMap::new();
    // F3-3 gate (g): adoção da sentinela `[LINA::CORRECTION]` medida desde o dia 1 — conta o
    // evento próprio `CorrectionObserved` no log (a porta de entrada da Mentality). Lição plan.md
    // (F1-3): medir uso REAL, não assumir adoção.
    let mut corrections = 0u64;

    for rec in records {
        match rec.kind.as_str() {
            "GoalDefined" | "GoalInterpreted" | "GoalConfirmed" | "GoalDecomposed"
            | "GoalAchieved" | "GoalEscalated" => {
                // Sem goal_id não há meta a que atribuir — conta e segue (tolerante).
                let Some(goal_id) = str_field(rec, "goal_id") else {
                    continue;
                };
                let agg = goals.entry(goal_id.to_string()).or_default();
                match rec.kind.as_str() {
                    "GoalDefined" => {
                        agg.defined = true;
                        if let Some(origin) = str_field(rec, "origin") {
                            agg.origin = Some(origin.to_string());
                        }
                    }
                    "GoalInterpreted" => agg.interpreted = true,
                    "GoalConfirmed" => agg.confirmed = true,
                    "GoalDecomposed" => agg.decomposed = true,
                    "GoalAchieved" => agg.achieved = true,
                    "GoalEscalated" => {
                        agg.escalated = true;
                        if let Some(reason) = str_field(rec, "reason") {
                            agg.escalate_reason = Some(reason.to_string());
                        }
                    }
                    _ => unreachable!("kind já filtrado pelo braço externo"),
                }
            }
            "EffortAssigned" => {
                effort_assigned += 1;
                // Default conservador igual ao do evento: effort=medium, origin=observed.
                let level = str_field(rec, "effort").unwrap_or("medium");
                let origin = str_field(rec, "origin").unwrap_or("observed");
                *effort_by_level.entry(level.to_string()).or_insert(0) += 1;
                *effort_by_origin.entry(origin.to_string()).or_insert(0) += 1;
            }
            // F3-3 gate (g): cada `CorrectionObserved` é uma correção do usuário captada pela
            // sentinela — a porta de entrada do aprendizado da Mentality.
            "CorrectionObserved" => corrections += 1,
            _ => {}
        }
    }

    // Deriva o funil varrendo as metas (cada meta conta no máximo 1 por marco).
    let mut goals_defined = 0u64;
    let mut goals_interpreted = 0u64;
    let mut goals_confirmed = 0u64;
    let mut goals_decomposed = 0u64;
    let mut goals_achieved = 0u64;
    let mut goals_escalated = 0u64;
    let mut escalation_reasons: BTreeMap<String, u64> = BTreeMap::new();
    let mut goals_by_origin: BTreeMap<String, u64> = BTreeMap::new();

    for agg in goals.values() {
        goals_defined += u64::from(agg.defined);
        goals_interpreted += u64::from(agg.interpreted);
        goals_confirmed += u64::from(agg.confirmed);
        goals_decomposed += u64::from(agg.decomposed);
        goals_achieved += u64::from(agg.achieved);
        goals_escalated += u64::from(agg.escalated);
        if let Some(origin) = &agg.origin {
            *goals_by_origin.entry(origin.clone()).or_insert(0) += 1;
        }
        if agg.escalated {
            // Escalada sem reason explícito (log degradado) ainda conta, sob rótulo honesto.
            let reason = agg
                .escalate_reason
                .clone()
                .unwrap_or_else(|| "(sem reason)".to_string());
            *escalation_reasons.entry(reason).or_insert(0) += 1;
        }
    }

    let close_denom = goals_achieved + goals_escalated;
    let goal_close_rate_pct = (close_denom > 0).then(|| {
        // 1 casa decimal — número de RELATÓRIO, não de comparação binária.
        (goals_achieved as f64 / close_denom as f64 * 1000.0).round() / 10.0
    });

    IntelligenceReport {
        goals_defined,
        goals_interpreted,
        goals_confirmed,
        goals_decomposed,
        goals_achieved,
        goals_escalated,
        goal_close_rate_pct,
        escalation_reasons,
        goals_by_origin,
        effort_assigned,
        effort_by_level,
        effort_by_origin,
        // F3-3 gate (g): contagem REAL de `CorrectionObserved` no log (a sentinela
        // `[LINA::CORRECTION]` já é evento próprio desde o contrato `bb52cae`). 0 quando não houve
        // nenhuma correção — nunca um número inventado.
        corrections,
    }
}

/// Lê o espelho canônico `<dir>/log.jsonl` (linhas malformadas são contadas e PULADAS — um log com
/// uma linha truncada por crash ainda rende relatório; nunca panic).
fn read_log(dir: &Path) -> Result<(Vec<EventRecord>, usize), String> {
    let path = dir.join("log.jsonl");
    let data = std::fs::read_to_string(&path).map_err(|e| {
        format!(
            "não li {} ({e}) — passe o diretório de eventos (<ws>/.lina/events)",
            path.display()
        )
    })?;
    let mut records = Vec::new();
    let mut skipped = 0usize;
    for line in data.lines().filter(|l| !l.trim().is_empty()) {
        match serde_json::from_str::<EventRecord>(line) {
            Ok(r) => records.push(r),
            Err(_) => skipped += 1,
        }
    }
    Ok((records, skipped))
}

/// `"a:1 · b:2"` ordenado (BTreeMap já ordena) — `(vazio)` quando não há nada a mostrar.
fn fmt_map(map: &BTreeMap<String, u64>) -> String {
    if map.is_empty() {
        return "(nenhum)".to_string();
    }
    map.iter()
        .map(|(k, v)| format!("{k}:{v}"))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Render humano do relatório (o `--json` serializa o struct cru para máquinas/testes).
fn render(report: &IntelligenceReport) -> String {
    let close = report.goal_close_rate_pct.map_or_else(
        || "n/a (nenhuma meta terminou)".to_string(),
        |r| format!("{r}%"),
    );
    format!(
        "adoção da inteligência F3 — MÉTRICA ACOMPANHADA (nunca gate binário; F3-CONF-3 R2)\n\
         GOAL (funil de metas, agrupado por goal_id):\n\
         · definidas: {} → interpretadas: {} → confirmadas (gate humano): {} → decompostas: {}\n\
         · fecharam (achieved): {} · escalaram: {} · taxa de fechamento: {close}\n\
         · razões de escala: {}\n\
         TRADUTOR (proveniência via GoalDefined.origin):\n\
         · por origem: {}\n\
         EFFORT (motor por papel, F3-0-4):\n\
         · atribuições: {} · por nível: {} · por origem: {}\n\
         CORREÇÃO/Mentality (sentinela [LINA::CORRECTION] → CorrectionObserved): {}",
        report.goals_defined,
        report.goals_interpreted,
        report.goals_confirmed,
        report.goals_decomposed,
        report.goals_achieved,
        report.goals_escalated,
        fmt_map(&report.escalation_reasons),
        fmt_map(&report.goals_by_origin),
        report.effort_assigned,
        fmt_map(&report.effort_by_level),
        fmt_map(&report.effort_by_origin),
        report.corrections,
    )
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let json = args.iter().any(|a| a == "--json");
    let Some(dir) = args.iter().find(|a| !a.starts_with("--")) else {
        eprintln!("uso: intelligence_adoption <events-dir> [--json]   (ex.: <ws>/.lina/events)");
        return ExitCode::from(2);
    };
    match read_log(Path::new(dir)) {
        Ok((records, skipped)) => {
            let report = intelligence_report(&records);
            if skipped > 0 {
                eprintln!("intelligence_adoption: {skipped} linha(s) malformada(s) puladas");
            }
            if json {
                // serializável por construção (struct plano) — fallback explícito p/ nunca panicar.
                println!(
                    "{}",
                    serde_json::to_string(&report).unwrap_or_else(|_| "{}".into())
                );
            } else {
                println!("{}", render(&report));
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("intelligence_adoption: {e}");
            ExitCode::from(2)
        }
    }
}

// ═══════════════════════════ testes (log SINTÉTICO conhecido — asserção exata) ═══════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    /// Registro sintético mínimo (mesmo shape do `log.jsonl`).
    fn rec(kind: &str, payload: serde_json::Value) -> EventRecord {
        EventRecord {
            seq: 0,
            ts: 0,
            kind: kind.to_string(),
            version: 1,
            payload,
        }
    }

    /// **Funil completo sobre log sintético conhecido (asserção EXATA).** Meta `g1` via `@Tradutor`
    /// percorre o funil inteiro (definida → interpretada → confirmada → decomposta → ACHIEVED); meta
    /// `g2` via `@Maestro` para no meio (definida → interpretada → ESCALADA `stalled`). Mais 3
    /// `EffortAssigned` (2 medium/assigned, 1 high/observed). Ruído alheio é ignorado.
    #[test]
    fn full_funnel_over_known_synthetic_log() {
        let log = vec![
            rec(
                "GoalDefined",
                serde_json::json!({"goal_id":"g1","origin":"@Tradutor","statement":"x"}),
            ),
            rec("GoalInterpreted", serde_json::json!({"goal_id":"g1"})),
            rec(
                "GoalConfirmed",
                serde_json::json!({"goal_id":"g1","by":"n1"}),
            ),
            rec("GoalDecomposed", serde_json::json!({"goal_id":"g1"})),
            rec("GoalAchieved", serde_json::json!({"goal_id":"g1"})),
            rec(
                "GoalDefined",
                serde_json::json!({"goal_id":"g2","origin":"@Maestro","statement":"y"}),
            ),
            rec("GoalInterpreted", serde_json::json!({"goal_id":"g2"})),
            rec(
                "GoalEscalated",
                serde_json::json!({"goal_id":"g2","reason":"stalled"}),
            ),
            rec(
                "EffortAssigned",
                serde_json::json!({"node":"n1","effort":"medium","origin":"assigned"}),
            ),
            rec(
                "EffortAssigned",
                serde_json::json!({"node":"n2","effort":"medium","origin":"assigned"}),
            ),
            rec(
                "EffortAssigned",
                serde_json::json!({"node":"n3","effort":"high","origin":"observed"}),
            ),
            // ruído que NÃO conta:
            rec("NodeAdded", serde_json::json!({"kind":"Terminal"})),
            rec("MessageRouted", serde_json::json!({"intent":"ask"})),
        ];
        let r = intelligence_report(&log);
        assert_eq!(
            r,
            IntelligenceReport {
                goals_defined: 2,
                goals_interpreted: 2,
                goals_confirmed: 1,
                goals_decomposed: 1,
                goals_achieved: 1,
                goals_escalated: 1,
                goal_close_rate_pct: Some(50.0),
                escalation_reasons: BTreeMap::from([("stalled".to_string(), 1)]),
                goals_by_origin: BTreeMap::from([
                    ("@Maestro".to_string(), 1),
                    ("@Tradutor".to_string(), 1),
                ]),
                effort_assigned: 3,
                effort_by_level: BTreeMap::from([
                    ("high".to_string(), 1),
                    ("medium".to_string(), 2),
                ]),
                effort_by_origin: BTreeMap::from([
                    ("assigned".to_string(), 2),
                    ("observed".to_string(), 1),
                ]),
                corrections: 0,
            }
        );
    }

    /// A taxa de fechamento é `null` (N/A) enquanto nenhuma meta terminou — nunca `0/0` nem panic.
    /// Métrica acompanhada não inventa número onde não há comportamento a acompanhar.
    #[test]
    fn close_rate_na_until_a_goal_terminates() {
        let in_flight = vec![
            rec(
                "GoalDefined",
                serde_json::json!({"goal_id":"g1","origin":"@Maestro"}),
            ),
            rec(
                "GoalConfirmed",
                serde_json::json!({"goal_id":"g1","by":"n1"}),
            ),
        ];
        let r = intelligence_report(&in_flight);
        assert_eq!(r.goals_defined, 1);
        assert_eq!(r.goals_confirmed, 1);
        assert_eq!(r.goals_achieved, 0);
        assert_eq!(r.goal_close_rate_pct, None);
        assert!(render(&r).contains("n/a"), "render explica o N/A");
        // Log vazio também é N/A, nunca panic.
        assert_eq!(intelligence_report(&[]).goal_close_rate_pct, None);
    }

    /// Funil agrupa por `goal_id`: eventos repetidos da MESMA meta (replay) não inflam o número —
    /// 1 meta = 1, mesmo com dois `GoalDefined`/`GoalAchieved` idênticos.
    #[test]
    fn replay_of_same_goal_does_not_inflate() {
        let log = vec![
            rec(
                "GoalDefined",
                serde_json::json!({"goal_id":"g1","origin":"@Tradutor"}),
            ),
            rec(
                "GoalDefined",
                serde_json::json!({"goal_id":"g1","origin":"@Tradutor"}),
            ),
            rec("GoalAchieved", serde_json::json!({"goal_id":"g1"})),
            rec("GoalAchieved", serde_json::json!({"goal_id":"g1"})),
        ];
        let r = intelligence_report(&log);
        assert_eq!(r.goals_defined, 1, "meta repetida conta 1");
        assert_eq!(r.goals_achieved, 1);
        assert_eq!(r.goal_close_rate_pct, Some(100.0));
        assert_eq!(
            r.goals_by_origin,
            BTreeMap::from([("@Tradutor".to_string(), 1)])
        );
    }

    /// Tolerância a log degradado: evento Goal SEM `goal_id` é pulado (não cria meta-fantasma);
    /// `EffortAssigned` sem campos cai nos defaults conservadores (`medium`/`observed`); escala sem
    /// `reason` conta sob rótulo honesto `(sem reason)`. Nunca panic.
    #[test]
    fn tolerant_to_missing_fields() {
        let log = vec![
            rec("GoalDefined", serde_json::json!({"origin":"@Maestro"})), // sem goal_id → pulado
            rec("GoalInterpreted", serde_json::json!({})),                // sem goal_id → pulado
            rec("EffortAssigned", serde_json::json!({"node":"n1"})),      // sem effort/origin
            rec(
                "GoalDefined",
                serde_json::json!({"goal_id":"g9"}), // sem origin → não classifica proveniência
            ),
            rec("GoalEscalated", serde_json::json!({"goal_id":"g9"})), // sem reason
        ];
        let r = intelligence_report(&log);
        assert_eq!(r.goals_defined, 1, "só a meta com goal_id conta");
        assert!(
            r.goals_by_origin.is_empty(),
            "meta sem origin não inventa proveniência"
        );
        assert_eq!(r.effort_assigned, 1);
        assert_eq!(
            r.effort_by_level,
            BTreeMap::from([("medium".to_string(), 1)])
        );
        assert_eq!(
            r.effort_by_origin,
            BTreeMap::from([("observed".to_string(), 1)])
        );
        assert_eq!(
            r.escalation_reasons,
            BTreeMap::from([("(sem reason)".to_string(), 1)])
        );
    }

    /// O render humano carrega o rótulo anti-gate e o slot futuro — o texto é parte do contrato
    /// (o número precisa circular como métrica acompanhada, não como critério binário).
    #[test]
    fn render_labels_metric_as_non_gate_and_future_slot() {
        let out = render(&intelligence_report(&[]));
        assert!(out.contains("MÉTRICA ACOMPANHADA"), "{out}");
        assert!(out.contains("nunca gate"), "{out}");
        assert!(out.contains("F3-3"), "{out}");
    }
}
