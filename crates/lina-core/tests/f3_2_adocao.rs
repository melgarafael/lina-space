//! **F3-2 · Métrica de ADOÇÃO do loop nativo (risco #3 da fase: "adoção 0%").**
//!
//! A peça `onda-f3-2.md` §Riscos crava: *"Adoção 0% → métrica de uso de `lina goal`/Tradutor no log
//! desde o dia 1."* Um gate que ASSUME adoção é cego; este módulo dá a ferramenta para MEDIR.
//!
//! [`adoption_metrics`] é uma função PURA sobre o event log (invariante #4: o log é a verdade) que
//! conta os sinais de uso real do loop do Maestro nativo:
//!   - `goal_defined` — quantas metas nasceram (uso do verbo `lina goal define`);
//!   - `via_tradutor` — quantas nasceram com `origin == "@Tradutor"` (adoção do papel Tradutor, F3-2-1);
//!   - `interpreted`/`confirmed` — progressão pelo gate humano (o loop sendo usado de ponta a ponta);
//!   - `team_spawns` — `SpawnRequested` atado a uma `goal_id` (montagem de time pelo loop, F3-2-3/5);
//!   - `achieved`/`escalated` — fechamentos (o termostato chegou ao fim).
//!
//! Dois testes: um DETERMINÍSTICO prova que a contagem está correta (a ferramenta é confiável); um
//! `#[ignore]` aponta a métrica para o LOG VIVO do Espaço e imprime o relatório de adoção — é o que o
//! gate roda sob demanda:
//! `cargo test -p lina-core --test f3_2_adocao adocao_no_log_vivo -- --ignored --nocapture`
//! (caminho do log via env `LINA_ADOCAO_LOG`, senão tenta `.lina/events/log.jsonl`).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use lina_core::{DomainEvent, EventRecord, EventStore, NodeId};

// ───────────────────────────── a métrica (função pura sobre o log) ─────────────────────────────

/// Sinais de adoção do loop nativo, contados do event log. Tudo derivável por replay — nenhuma
/// estrutura mutável de 1ª ordem (invariante #4).
#[derive(Debug, Default, PartialEq, Eq)]
struct Adoption {
    goal_defined: usize,
    via_tradutor: usize,
    interpreted: usize,
    confirmed: usize,
    team_spawns: usize,
    achieved: usize,
    escalated: usize,
}

impl Adoption {
    /// `true` se há QUALQUER sinal de uso do loop — o gate distingue "ninguém usou" de "usaram".
    fn any_usage(&self) -> bool {
        self.goal_defined > 0
    }

    fn report(&self) -> String {
        format!(
            "ADOÇÃO do loop nativo (F3-2):\n\
             • metas definidas (lina goal):      {}\n\
             • via Tradutor (origin=@Tradutor):  {}\n\
             • interpretadas:                    {}\n\
             • confirmadas (gate humano):        {}\n\
             • spawns de time (montagem):        {}\n\
             • fechadas (achieved):              {}\n\
             • escaladas ao humano:              {}\n\
             → uso detectado: {}",
            self.goal_defined,
            self.via_tradutor,
            self.interpreted,
            self.confirmed,
            self.team_spawns,
            self.achieved,
            self.escalated,
            if self.any_usage() {
                "SIM"
            } else {
                "NÃO (adoção 0%)"
            },
        )
    }
}

/// Conta os sinais de adoção varrendo o log por `kind`/payload — barato, sem desserializar tipado
/// (mesmo padrão de `CostLedger`: `kind != "…" → pula`). `origin`/`goal_id` lidos do payload.
fn adoption_metrics(events: &[EventRecord]) -> Adoption {
    let mut a = Adoption::default();
    for rec in events {
        match rec.kind.as_str() {
            "GoalDefined" => {
                a.goal_defined += 1;
                if rec.payload.get("origin").and_then(|v| v.as_str()) == Some("@Tradutor") {
                    a.via_tradutor += 1;
                }
            }
            "GoalInterpreted" => a.interpreted += 1,
            "GoalConfirmed" => a.confirmed += 1,
            // montagem de time = um spawn atado a uma meta (o loop, não um spawn avulso de fork-bomb).
            "SpawnRequested" => {
                let has_goal = rec
                    .payload
                    .get("goal_id")
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| !s.is_empty());
                if has_goal {
                    a.team_spawns += 1;
                }
            }
            "GoalAchieved" => a.achieved += 1,
            "GoalEscalated" => a.escalated += 1,
            _ => {}
        }
    }
    a
}

// ───────────────────────────── substrato ─────────────────────────────

static SEQ: AtomicU64 = AtomicU64::new(0);

fn unique(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("lina-f32adoc-{tag}-{}-{n}", std::process::id()))
}

// ════════════════════════════════════ a métrica é confiável (determinístico) ════════════════════════════════════

/// A ferramenta conta CERTO: um log com 2 metas (1 via Tradutor), 1 interpretada/confirmada, 1 spawn de
/// time atado à meta e 1 spawn avulso (ignorado), 1 fechada. Prova cada campo da [`Adoption`].
///
/// QUEBRA SE… a métrica contasse spawn avulso como montagem de time, ou não filtrasse `origin` —
/// `team_spawns`/`via_tradutor` divergiriam dos valores afirmados.
#[test]
fn adocao_conta_sinais_do_loop() {
    let tmp = unique("conta");
    let _ = std::fs::remove_dir_all(&tmp);
    let mut store = EventStore::open(tmp.join("events")).expect("store");

    store
        .append(&DomainEvent::GoalDefined {
            goal_id: "g1".into(),
            statement: "criar landing".into(),
            root_cause_id: "r1".into(),
            origin: "@Tradutor".into(), // ← nasceu via Tradutor.
            budget_tokens: 0,
        })
        .unwrap();
    store
        .append(&DomainEvent::GoalDefined {
            goal_id: "g2".into(),
            statement: "outra meta".into(),
            root_cause_id: "r2".into(),
            origin: "@Maestro".into(), // ← degradou ao Maestro (sem Tradutor).
            budget_tokens: 0,
        })
        .unwrap();
    store
        .append(&DomainEvent::GoalInterpreted {
            goal_id: "g1".into(),
            interpretation: "LP simples".into(),
            strategy: "1 dev".into(),
            proposed_team: vec!["Frontend".into()],
            acceptance_criteria: vec![],
        })
        .unwrap();
    store
        .append(&DomainEvent::GoalConfirmed {
            goal_id: "g1".into(),
            by: NodeId::from_u128(1),
            amended_statement: None,
        })
        .unwrap();
    // spawn de time (montagem): atado à goal_id.
    store
        .append(&DomainEvent::SpawnRequested {
            id: "msg-spawn-1".into(),
            requested_by: NodeId::from_u128(1),
            name: "@Front".into(),
            role: "Frontend".into(),
            root_cause_id: "r1".into(),
            hops: 0,
            prompt: String::new(),
            model: None,
            effort: None,
            goal_id: Some("g1".into()),
        })
        .unwrap();
    // spawn AVULSO (sem goal_id): NÃO conta como montagem de time do loop.
    store
        .append(&DomainEvent::SpawnRequested {
            id: "msg-spawn-2".into(),
            requested_by: NodeId::from_u128(2),
            name: "@X".into(),
            role: "Dev".into(),
            root_cause_id: "r9".into(),
            hops: 0,
            prompt: String::new(),
            model: None,
            effort: None,
            goal_id: None,
        })
        .unwrap();
    store
        .append(&DomainEvent::GoalAchieved {
            goal_id: "g1".into(),
            iterations: 1,
            verdict_event_id: "e".into(),
        })
        .unwrap();

    let a = adoption_metrics(&store.events().unwrap());
    assert_eq!(
        a,
        Adoption {
            goal_defined: 2,
            via_tradutor: 1,
            interpreted: 1,
            confirmed: 1,
            team_spawns: 1, // só o atado à goal; o avulso é ignorado.
            achieved: 1,
            escalated: 0,
        },
        "a métrica conta cada sinal de adoção corretamente"
    );
    assert!(a.any_usage(), "uso detectado");
    let _ = std::fs::remove_dir_all(&tmp);
}

/// Log VAZIO (ninguém usou o loop) → adoção 0% honesta (sem falso-positivo de uso).
#[test]
fn adocao_zero_quando_ninguem_usou() {
    let a = adoption_metrics(&[]);
    assert_eq!(a, Adoption::default());
    assert!(
        !a.any_usage(),
        "log vazio = adoção 0% (o gate não inventa uso)"
    );
}

// ════════════════════════════════════ ferramenta do gate: log VIVO ════════════════════════════════════

/// Lê o log.jsonl (espelho append-only) — uma linha por `EventRecord` (mesmo formato de `repro_w4`).
fn read_jsonl(path: &std::path::Path) -> Vec<EventRecord> {
    let Ok(data) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    data.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<EventRecord>(l).ok())
        .collect()
}

/// **Ferramenta do gate (não roda na suíte normal — `#[ignore]`).** Aponta a métrica para o LOG VIVO do
/// Espaço e imprime o relatório de adoção. Caminho via env `LINA_ADOCAO_LOG`, senão tenta os locais
/// usuais. Não falha por "adoção baixa" (é medição, não setpoint) — só por log ilegível quando apontado.
///
/// Uso: `cargo test -p lina-core --test f3_2_adocao adocao_no_log_vivo -- --ignored --nocapture`
#[test]
#[ignore = "ferramenta de gate: mede o log vivo sob demanda (estado não-determinístico)"]
fn adocao_no_log_vivo() {
    let candidates: Vec<PathBuf> = std::env::var("LINA_ADOCAO_LOG")
        .ok()
        .map(PathBuf::from)
        .into_iter()
        .chain([
            PathBuf::from(".lina/events/log.jsonl"),
            PathBuf::from("../../.lina/events/log.jsonl"),
        ])
        .collect();

    let found = candidates.iter().find(|p| p.exists());
    match found {
        Some(path) => {
            let recs = read_jsonl(path);
            let a = adoption_metrics(&recs);
            println!(
                "\n[log: {}  ({} eventos)]\n{}",
                path.display(),
                recs.len(),
                a.report()
            );
        }
        None => {
            println!(
                "\n[adoção] nenhum log.jsonl encontrado em {candidates:?} — \
                 aponte com LINA_ADOCAO_LOG=<caminho>. Sem dados ≠ adoção 0%."
            );
        }
    }
}
