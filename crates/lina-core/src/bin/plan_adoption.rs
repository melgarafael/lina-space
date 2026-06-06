//! `plan_adoption` — **F1-0-9 (c): telemetria de ADOÇÃO do plano compartilhado**.
//!
//! Leitor **offline** do `log.jsonl` canônico: mede quanta coordenação passou pelo plano
//! (`plan.claim`/`plan.check` — W3-5) versus delegação **ad-hoc** (`ask`/`handoff` direto),
//! agrupando a delegação por `root_cause_id` (uma CADEIA de coordenação = 1 sequência, não N
//! mensagens — re-asks e saltos da mesma cadeia não inflam o denominador).
//!
//! **É MÉTRICA ACOMPANHADA, NUNCA GATE** (story F1-0-9, reformulada pós-auditoria): adoção é
//! comportamento de LLM, não-determinístico — o que se gateia são os freios/verbos/doutrina.
//! Este leitor não tem modo "falha": ele REPORTA (baseline das 14h: 0%). A saída diz isso
//! explicitamente para o número nunca virar critério binário por engano.
//!
//! ## Decisões de design (registradas — fronteira da rodada E4)
//! - **Bin auto-descoberto** (`src/bin/`): zero edição em `lib.rs`/`Cargo.toml`/módulos com dono
//!   (multi-terminal). Mesmo padrão do `baseline_f1_0`.
//! - **Lê o log EXISTENTE** — nenhum evento novo: `PlanClaimed`/`PlanChecked` (W3-5) e
//!   `MessageRouted{intent, root_cause_id}` (W3-4/W3-7) bastam. `PlanClaimed` não carrega
//!   `root_cause_id`, então cadeias MISTAS (ask que virou claim) não são correlacionáveis —
//!   se o gate quiser isso um dia, é campo novo no evento (linha via Maestro), não inferência.
//! - **Cadeia ad-hoc** = `root_cause_id` distinto entre os `MessageRouted` de `ask`/`handoff`
//!   (vazio → o próprio `id` da mensagem: origem de turno é a própria cadeia). `reply`/`status`/
//!   `handshake`/`broadcast` não são atos novos de coordenação e ficam fora.
//! - **Taxa** = claims / (claims + cadeias ad-hoc). Log sem coordenação → taxa `null` (N/A),
//!   nunca 0/0 nem panic.
//!
//! Uso: `plan_adoption <events-dir> [--json]` (ex.: `<ws>/.lina/events`).

use std::path::Path;
use std::process::ExitCode;

use lina_core::EventRecord;

/// O relatório de adoção — números crus + taxa derivada (None = log sem coordenação).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct AdoptionReport {
    /// Atos de distribuição via plano (`PlanClaimed`).
    pub plan_claims: u64,
    /// Conclusões via plano (`PlanChecked`) — informativo (não entra na taxa).
    pub plan_checks: u64,
    /// Cadeias de delegação ad-hoc (`root_cause_id` distintos em `ask`/`handoff` roteados).
    pub ask_chains: u64,
    /// Mensagens ad-hoc roteadas no total (contexto do tamanho das cadeias).
    pub ask_msgs: u64,
    /// `claims / (claims + cadeias)` em %, com 1 casa. `None` = sem coordenação no log.
    pub rate_pct: Option<f64>,
}

/// Computa o relatório sobre os registros do log — **pura** (testável com log sintético).
#[must_use]
pub fn adoption_report(records: &[EventRecord]) -> AdoptionReport {
    let mut plan_claims = 0u64;
    let mut plan_checks = 0u64;
    let mut ask_msgs = 0u64;
    let mut chains: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    for rec in records {
        match rec.kind.as_str() {
            "PlanClaimed" => plan_claims += 1,
            "PlanChecked" => plan_checks += 1,
            "MessageRouted" => {
                let intent = rec
                    .payload
                    .get("intent")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                // Atos NOVOS de coordenação ad-hoc: ask/handoff. reply/status/handshake/broadcast
                // não abrem trabalho novo (ver doc do módulo).
                if intent == "ask" || intent == "handoff" {
                    ask_msgs += 1;
                    let root = rec
                        .payload
                        .get("root_cause_id")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty())
                        .or_else(|| rec.payload.get("id").and_then(|v| v.as_str()))
                        .unwrap_or_default();
                    chains.insert(root.to_string());
                }
            }
            _ => {}
        }
    }

    let ask_chains = chains.len() as u64;
    let denom = plan_claims + ask_chains;
    let rate_pct = (denom > 0).then(|| {
        // 1 casa decimal — número de RELATÓRIO, não de comparação binária.
        (plan_claims as f64 / denom as f64 * 1000.0).round() / 10.0
    });
    AdoptionReport {
        plan_claims,
        plan_checks,
        ask_chains,
        ask_msgs,
        rate_pct,
    }
}

/// Lê o espelho canônico `<dir>/log.jsonl` (linhas malformadas são contadas e PULADAS — um log
/// com uma linha truncada por crash ainda rende relatório; nunca panic).
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

/// Render humano do relatório (o `--json` serializa o struct cru para máquinas/testes).
fn render(report: &AdoptionReport) -> String {
    let rate = report.rate_pct.map_or_else(
        || "n/a (sem coordenação no log)".to_string(),
        |r| format!("{r}%"),
    );
    format!(
        "adoção do plano compartilhado — MÉTRICA ACOMPANHADA (nunca gate binário; F1-0-9)\n\
         · plan.claim: {} · plan.check: {}\n\
         · cadeias ask/handoff ad-hoc (root_cause distintos): {} ({} msg(s))\n\
         · taxa plan-vs-ask: {rate}",
        report.plan_claims, report.plan_checks, report.ask_chains, report.ask_msgs
    )
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let json = args.iter().any(|a| a == "--json");
    let Some(dir) = args.iter().find(|a| !a.starts_with("--")) else {
        eprintln!("uso: plan_adoption <events-dir> [--json]   (ex.: <ws>/.lina/events)");
        return ExitCode::from(2);
    };
    match read_log(Path::new(dir)) {
        Ok((records, skipped)) => {
            let report = adoption_report(&records);
            if skipped > 0 {
                eprintln!("plan_adoption: {skipped} linha(s) malformada(s) puladas");
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
            eprintln!("plan_adoption: {e}");
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

    /// **Critério 3 da story (asserção EXATA sobre log sintético):** 2 claims + 1 check + 2
    /// cadeias ad-hoc (3 asks: duas da MESMA cadeia + 1 de outra) → taxa = 2/(2+2) = 50.0%.
    #[test]
    fn exact_rate_over_known_synthetic_log() {
        let log = vec![
            rec("PlanClaimed", serde_json::json!({"item":"T1","by":"@A"})),
            rec("PlanClaimed", serde_json::json!({"item":"T2","by":"@B"})),
            rec("PlanChecked", serde_json::json!({"item":"T1","by":"@A"})),
            // cadeia 1: duas msgs com o MESMO root → 1 sequência ad-hoc.
            rec(
                "MessageRouted",
                serde_json::json!({"id":"m1","intent":"ask","root_cause_id":"rc-1"}),
            ),
            rec(
                "MessageRouted",
                serde_json::json!({"id":"m2","intent":"ask","root_cause_id":"rc-1"}),
            ),
            // cadeia 2: handoff também é coordenação ad-hoc.
            rec(
                "MessageRouted",
                serde_json::json!({"id":"m3","intent":"handoff","root_cause_id":"rc-2"}),
            ),
            // ruído que NÃO conta: reply/handshake/broadcast + eventos alheios.
            rec(
                "MessageRouted",
                serde_json::json!({"id":"m4","intent":"reply","root_cause_id":"rc-1"}),
            ),
            rec("Handshake", serde_json::json!({"name":"@A"})),
            rec("NodeAdded", serde_json::json!({"kind":"Terminal"})),
        ];
        let r = adoption_report(&log);
        assert_eq!(
            r,
            AdoptionReport {
                plan_claims: 2,
                plan_checks: 1,
                ask_chains: 2,
                ask_msgs: 3,
                rate_pct: Some(50.0),
            }
        );
    }

    /// Baseline do DIRECIONAMENTO (14h, zero `lina plan`): só asks → taxa 0.0% — o número que o
    /// relatório do gate compara. E o caso inverso: só plano → 100%.
    #[test]
    fn baseline_zero_and_full_adoption() {
        let only_asks = vec![rec(
            "MessageRouted",
            serde_json::json!({"id":"m1","intent":"ask","root_cause_id":"rc-1"}),
        )];
        assert_eq!(adoption_report(&only_asks).rate_pct, Some(0.0));

        let only_plan = vec![rec(
            "PlanClaimed",
            serde_json::json!({"item":"T1","by":"@A"}),
        )];
        assert_eq!(adoption_report(&only_plan).rate_pct, Some(100.0));
    }

    /// Log sem NENHUMA coordenação → taxa N/A (`None`), nunca 0/0 nem panic — métrica
    /// acompanhada não inventa número onde não há comportamento a acompanhar.
    #[test]
    fn empty_coordination_yields_na_not_zero() {
        let log = vec![rec("NodeAdded", serde_json::json!({"kind":"Terminal"}))];
        let r = adoption_report(&log);
        assert_eq!(r.rate_pct, None);
        assert!(render(&r).contains("n/a"), "render explica o N/A");
        assert_eq!(adoption_report(&[]).rate_pct, None);
    }

    /// `root_cause_id` VAZIO (origem de turno / logs antigos) → a cadeia é a própria mensagem
    /// (`id`): duas origens distintas = 2 cadeias; nunca colapsam numa cadeia-fantasma "".
    #[test]
    fn empty_root_cause_falls_back_to_message_id() {
        let log = vec![
            rec(
                "MessageRouted",
                serde_json::json!({"id":"m1","intent":"ask","root_cause_id":""}),
            ),
            rec(
                "MessageRouted",
                serde_json::json!({"id":"m2","intent":"ask"}),
            ),
        ];
        let r = adoption_report(&log);
        assert_eq!(r.ask_chains, 2, "cada origem sem root é a própria cadeia");
    }

    /// O render humano carrega o rótulo anti-gate (a story exige que o número circule como
    /// métrica acompanhada — o texto é parte do contrato).
    #[test]
    fn render_labels_metric_as_non_gate() {
        let r = adoption_report(&[]);
        let out = render(&r);
        assert!(out.contains("MÉTRICA ACOMPANHADA"), "{out}");
        assert!(out.contains("nunca gate"), "{out}");
    }
}
