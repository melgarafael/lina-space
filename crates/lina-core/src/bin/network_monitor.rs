//! `network_monitor` — **F4-0-5 (gate d): monitor de rede HONESTO** + **F4-0-6: catálogo por trust-tier**.
//!
//! Leitor **offline** do `log.jsonl` canônico. Não é um sniffer de pacotes: é uma **afirmação
//! honesta e auditável** da exposição de rede do Espaço (doc 40 §2 — a honestidade é o produto).
//! Projeta, do event log, quais canais estão DECLARADOS (`ChannelRegistered`) e quais estão
//! CONECTADOS (`ChannelConnected`), e afirma quantas conexões de saída são ESPERADAS do Lina.
//!
//! ## Gate (d) — prova local-first por AUSÊNCIA
//! Registrar ≠ conectar (default-deny por pertencimento, ADR 0006): um canal nasce "declarado, não
//! conectado". Enquanto **0 canais conectados**, o Lina **não abre socket de saída** —
//! `expected_outbound = 0`. Como `ChannelConnected` é F4-1+ (ainda não existe no contrato de
//! `events.rs`), HOJE todo canal está declarado-não-conectado ⇒ 0 conexão de saída esperada. O
//! monitor afirma isso e diz COMO conferir POR FORA — a prova não vive no nosso código, vive no
//! `nettop`/`lsof` do SO (rode com o Lina aberto):
//!
//! ```text
//! nettop -P -l 1 | grep -i lina          # macOS: 0 linhas = 0 conexão de saída do Lina
//! lsof -i -a -p <pid-do-processo-Lina>   # 0 conexões de saída enquanto 0 canais conectados
//! ```
//!
//! O monitor é HONESTO nos dois sentidos: com 0 conectados afirma 0; quando HOUVER um conectado
//! (F4-1+), afirma o número REAL — nunca um "tudo zero" que mente (um relatório que sempre diz 0 é
//! pior que vermelho). O teste `connected_channel_is_counted_as_outbound` trava essa honestidade.
//!
//! ## Catálogo (F4-0-6, gate e) — curadoria = segurança para o leigo
//! `trust_tier ∈ {core, curado, comunidade}` (doc 40 §6/§11): **core** auto-disponível (parte do
//! produto), **curado** aprovado por PR (estar no catálogo = revisado), **comunidade** opt-in
//! explícito. O catálogo agrupa os canais registrados por tier, com o `install_ref` PINADO visível.
//! O tier ORDENA a curadoria, **nunca autoriza** um efeito externo (isso é o broker, F4-0-3 —
//! manifesto/tier é DADO, jamais autoridade). A validação de SCHEMA do manifesto em merge-time
//! (gate e) vive no passo de CI (`ci.yml`), reusando o parser de `channel.rs` (F4-0-1).
//!
//! ## Decisões de design
//! - **Bin auto-descoberto** (`src/bin/`): zero edição em `lib.rs`/`Cargo.toml` (multi-terminal).
//!   Mesmo padrão de [`plan_adoption`]/`intelligence_adoption`.
//! - **Read-only, ZERO mutação, ZERO evento novo:** só lê o log e projeta (inv #4: projeção é
//!   derivada reconstruível do log).
//! - **Lê campos do `payload` por chave** (`payload.get(...)`): tolerante a versão — campo ausente
//!   vira rótulo honesto, nunca panic (regra "conte e siga").
//! - **Projeção por canal = padrão `ClueSet`** (o último `ChannelRegistered` de um `channel` vence
//!   nos campos declarativos; o estado de conexão é preservado entre re-registros); replay idempotente.
//! - **`channels_declared`** = canais vistos no log (no fluxo normal, todos vêm de `ChannelRegistered`
//!   — conectar pressupõe registrar). O ciclo de DESconexão é F4-1+; quando o evento existir, basta
//!   um braço a mais aqui.
//!
//! Uso: `network_monitor <events-dir> [--json]`   (ex.: `<ws>/.lina/events`).

use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;

use lina_core::EventRecord;

/// Tiers conhecidos, na ordem de curadoria (do mais confiável ao opt-in). O glossário é a tradução
/// honesta da curadoria para o leigo (doc 40 §6/§11). Tier fora desta lista cai em `[outros]`.
const TIER_ORDER: &[(&str, &str)] = &[
    ("core", "auto-disponível"),
    ("curado", "aprovado por PR"),
    ("comunidade", "opt-in explícito"),
];

/// Um canal projetado do log: identidade declarativa + curadoria + estado de conexão.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ChannelState {
    /// Nome do canal (chave no registro).
    pub channel: String,
    /// Tier de confiança (`core`/`curado`/`comunidade`); `(sem tier)` se o log não o trouxe.
    pub trust_tier: String,
    /// Ref de instalação PINADA (SHA/tag); `(sem ref)` se ausente no log.
    pub install_ref: String,
    /// Manifesto de origem (caminho relativo); informativo.
    pub manifest_ref: String,
    /// `true` só com um `ChannelConnected` (F4-1+). Hoje, sempre `false` ⇒ 0 saída esperada.
    pub connected: bool,
}

/// Afirmação de exposição de rede + catálogo de curadoria — números crus, sem inflar.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct NetworkExposureReport {
    /// Canais vistos no log (declarados via `ChannelRegistered`).
    pub channels_declared: u64,
    /// Canais conectados (`ChannelConnected`, F4-1+) — hoje 0.
    pub channels_connected: u64,
    /// Conexões de saída ESPERADAS atribuíveis ao Lina = nº de canais conectados. 0 conectados ⇒ 0
    /// (a prova local-first por ausência, gate (d)).
    pub expected_outbound: u64,
    /// Sumário da curadoria: `trust_tier` → nº de canais. Mostra a distribuição por tier.
    pub tier_counts: BTreeMap<String, u64>,
    /// Projeção plana, ordenada por nome (o catálogo agrupado é derivado disto no render).
    pub channels: Vec<ChannelState>,
}

/// Lê `payload.<key>` como `&str`, vazio/ausente → `None`. Tolerante a versão (chave nova/velha).
fn str_field<'a>(rec: &'a EventRecord, key: &str) -> Option<&'a str> {
    rec.payload
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.trim().is_empty())
}

/// Computa a afirmação de exposição sobre os registros do log — **pura** (testável com log sintético).
#[must_use]
pub fn network_exposure(records: &[EventRecord]) -> NetworkExposureReport {
    // Projeção por canal (padrão ClueSet: último registro vence; conexão preservada entre re-registros).
    let mut channels: BTreeMap<String, ChannelState> = BTreeMap::new();

    for rec in records {
        match rec.kind.as_str() {
            "ChannelRegistered" => {
                // Sem nome de canal não há a que atribuir o registro — conta e segue (tolerante).
                let Some(channel) = str_field(rec, "channel") else {
                    continue;
                };
                let entry = channels
                    .entry(channel.to_string())
                    .or_insert_with(|| ChannelState {
                        channel: channel.to_string(),
                        trust_tier: String::new(),
                        install_ref: String::new(),
                        manifest_ref: String::new(),
                        connected: false,
                    });
                // Campos declarativos: o último registro vence. `connected` NÃO é tocado aqui —
                // re-registrar (atualizar manifesto/tier) não desconecta.
                entry.trust_tier = str_field(rec, "trust_tier")
                    .unwrap_or("(sem tier)")
                    .to_string();
                entry.install_ref = str_field(rec, "install_ref")
                    .unwrap_or("(sem ref)")
                    .to_string();
                entry.manifest_ref = str_field(rec, "manifest_ref")
                    .unwrap_or_default()
                    .to_string();
            }
            // F4-1+: marca o canal como conectado. O evento ainda não existe em `events.rs`, então
            // este braço fica inerte HOJE (0 hits) — a porta para o ciclo de conexão sem reescrever
            // o monitor. Conectar sobre um canal não-registrado ainda conta como conexão real.
            "ChannelConnected" => {
                let Some(channel) = str_field(rec, "channel") else {
                    continue;
                };
                channels
                    .entry(channel.to_string())
                    .or_insert_with(|| ChannelState {
                        channel: channel.to_string(),
                        trust_tier: "(sem tier)".to_string(),
                        install_ref: "(sem ref)".to_string(),
                        manifest_ref: String::new(),
                        connected: false,
                    })
                    .connected = true;
            }
            _ => {}
        }
    }

    let channels: Vec<ChannelState> = channels.into_values().collect();
    let channels_declared = channels.len() as u64;
    let channels_connected = channels.iter().filter(|c| c.connected).count() as u64;
    // Conexão de saída atribuível ao Lina = canal conectado. É a afirmação do gate (d).
    let expected_outbound = channels_connected;

    let mut tier_counts: BTreeMap<String, u64> = BTreeMap::new();
    for c in &channels {
        *tier_counts.entry(c.trust_tier.clone()).or_insert(0) += 1;
    }

    NetworkExposureReport {
        channels_declared,
        channels_connected,
        expected_outbound,
        tier_counts,
        channels,
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

/// `"<nome> @ <install_ref>"` por canal, ` · ` entre eles; `(nenhum)` quando o tier está vazio.
fn fmt_channels(channels: &[&ChannelState]) -> String {
    if channels.is_empty() {
        return "(nenhum)".to_string();
    }
    channels
        .iter()
        .map(|c| format!("{} @ {}", c.channel, c.install_ref))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Catálogo agrupado por tier, na ordem de curadoria (`core`→`curado`→`comunidade`), com um grupo
/// `[outros]` para tiers fora do vocabulário (log degradado / tier futuro).
fn catalog_section(channels: &[ChannelState]) -> String {
    let mut out = String::from("CATÁLOGO DE CANAIS (curadoria = segurança; F4-0-6):\n");
    for (tier, gloss) in TIER_ORDER {
        let in_tier: Vec<&ChannelState> =
            channels.iter().filter(|c| c.trust_tier == *tier).collect();
        out.push_str(&format!(
            "  [{tier}] ({gloss}): {}\n",
            fmt_channels(&in_tier)
        ));
    }
    let others: Vec<&ChannelState> = channels
        .iter()
        .filter(|c| !TIER_ORDER.iter().any(|(t, _)| *t == c.trust_tier))
        .collect();
    if !others.is_empty() {
        out.push_str(&format!("  [outros]: {}\n", fmt_channels(&others)));
    }
    out
}

/// Render humano da afirmação (o `--json` serializa o struct cru para máquinas/o gate de QA).
fn render(report: &NetworkExposureReport) -> String {
    format!(
        "exposição de rede do Espaço — AFIRMAÇÃO HONESTA (local-first por ausência; F4-0-5)\n\
         · canais declarados: {} · conectados: {}\n\
         · conexões de saída esperadas atribuíveis ao Lina: {}\n\
         {}\
         VERIFICAÇÃO EXTERNA (rode com o Lina aberto — a prova vive no SO, não neste código):\n\
         · nettop -P -l 1 | grep -i lina          → 0 linha = 0 conexão de saída do Lina\n\
         · lsof -i -a -p <pid-do-processo-Lina>   → 0 conexão enquanto 0 canais conectados\n\
         {}",
        report.channels_declared,
        report.channels_connected,
        report.expected_outbound,
        outbound_verdict(report),
        catalog_section(&report.channels),
    )
}

/// A frase-veredito do gate (d): traduz o número em afirmação honesta para quem lê.
fn outbound_verdict(report: &NetworkExposureReport) -> String {
    if report.expected_outbound == 0 {
        "  → 0 canal conectado ⇒ NENHUM socket de saída do Lina é esperado.\n".to_string()
    } else {
        format!(
            "  → {} canal(is) conectado(s) ⇒ o Lina PODE abrir saída por eles (confira por canal abaixo).\n",
            report.expected_outbound
        )
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let json = args.iter().any(|a| a == "--json");
    let Some(dir) = args.iter().find(|a| !a.starts_with("--")) else {
        eprintln!("uso: network_monitor <events-dir> [--json]   (ex.: <ws>/.lina/events)");
        return ExitCode::from(2);
    };
    match read_log(Path::new(dir)) {
        Ok((records, skipped)) => {
            let report = network_exposure(&records);
            if skipped > 0 {
                eprintln!("network_monitor: {skipped} linha(s) malformada(s) puladas");
            }
            if json {
                // serializável por construção (struct plano) — fallback explícito p/ nunca panicar.
                println!(
                    "{}",
                    serde_json::to_string(&report).unwrap_or_else(|_| "{}".into())
                );
            } else {
                print!("{}", render(&report));
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("network_monitor: {e}");
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

    /// Um `ChannelRegistered` no shape do payload REAL (`#[serde(tag="event")]` achata os campos no
    /// topo + injeta `"event"`). Espelha o que o `EventStore::append` grava.
    fn registered(channel: &str, tier: &str, install_ref: &str) -> EventRecord {
        rec(
            "ChannelRegistered",
            serde_json::json!({
                "event": "ChannelRegistered",
                "channel": channel,
                "manifest_ref": format!("channels/{channel}/manifest.toml"),
                "trust_tier": tier,
                "install_ref": install_ref,
            }),
        )
    }

    /// **Gate (d) baseline (asserção EXATA):** canais DECLARADOS, 0 conectados ⇒ `expected_outbound`
    /// = 0. É o estado real HOJE (registrar ≠ conectar; `ChannelConnected` é F4-1+). 2 canais em
    /// tiers distintos exercem o catálogo.
    #[test]
    fn declared_not_connected_is_zero_outbound() {
        let log = vec![
            registered("slack", "core", "sha-aaa"),
            registered("zap", "comunidade", "v1.2.3"),
            // ruído alheio que NÃO conta:
            rec("NodeAdded", serde_json::json!({"kind": "Terminal"})),
            rec("GoalDefined", serde_json::json!({"goal_id": "g1"})),
        ];
        let r = network_exposure(&log);
        assert_eq!(
            r,
            NetworkExposureReport {
                channels_declared: 2,
                channels_connected: 0,
                expected_outbound: 0,
                tier_counts: BTreeMap::from([
                    ("comunidade".to_string(), 1),
                    ("core".to_string(), 1),
                ]),
                channels: vec![
                    // ordenado por nome (BTreeMap): slack vem depois de zap? não — "slack" > "zap"? 's'<'z'.
                    ChannelState {
                        channel: "slack".to_string(),
                        trust_tier: "core".to_string(),
                        install_ref: "sha-aaa".to_string(),
                        manifest_ref: "channels/slack/manifest.toml".to_string(),
                        connected: false,
                    },
                    ChannelState {
                        channel: "zap".to_string(),
                        trust_tier: "comunidade".to_string(),
                        install_ref: "v1.2.3".to_string(),
                        manifest_ref: "channels/zap/manifest.toml".to_string(),
                        connected: false,
                    },
                ],
            }
        );
    }

    /// Log VAZIO (e log sem canais) → 0 declarados, 0 saída. Nunca panic, nunca número inventado.
    #[test]
    fn empty_log_is_zero_exposure() {
        let r = network_exposure(&[]);
        assert_eq!(r.channels_declared, 0);
        assert_eq!(r.channels_connected, 0);
        assert_eq!(r.expected_outbound, 0);
        assert!(r.channels.is_empty());
        assert!(r.tier_counts.is_empty());
    }

    /// **Honestidade nos DOIS sentidos (trava anti-mentira):** quando um canal está CONECTADO
    /// (`ChannelConnected`, simulando F4-1), o monitor afirma `expected_outbound = 1` — não esconde a
    /// saída sob um "tudo zero". Um relatório que sempre diz 0 é pior que vermelho.
    #[test]
    fn connected_channel_is_counted_as_outbound() {
        let log = vec![
            registered("slack", "core", "sha-aaa"),
            registered("zap", "curado", "v2.0.0"),
            rec("ChannelConnected", serde_json::json!({"channel": "slack"})),
        ];
        let r = network_exposure(&log);
        assert_eq!(r.channels_declared, 2);
        assert_eq!(r.channels_connected, 1);
        assert_eq!(r.expected_outbound, 1);
        let slack = r.channels.iter().find(|c| c.channel == "slack").unwrap();
        assert!(slack.connected, "slack está conectado");
        let zap = r.channels.iter().find(|c| c.channel == "zap").unwrap();
        assert!(!zap.connected, "zap segue declarado-não-conectado");
        // O render NÃO mente: noticia a saída.
        assert!(render(&r).contains("PODE abrir saída"), "{}", render(&r));
    }

    /// Padrão `ClueSet`: re-registrar o MESMO canal (mudou de tier/ref) não cria um segundo canal —
    /// o último registro vence, e a conexão é PRESERVADA entre re-registros. Replay idempotente.
    #[test]
    fn last_registration_wins_and_preserves_connection() {
        let log = vec![
            registered("slack", "comunidade", "sha-old"),
            rec("ChannelConnected", serde_json::json!({"channel": "slack"})),
            // re-registro promove o tier e fixa nova ref — sem desconectar.
            registered("slack", "core", "sha-new"),
        ];
        let r = network_exposure(&log);
        assert_eq!(r.channels_declared, 1, "1 canal, não 2");
        assert_eq!(r.channels.len(), 1);
        let slack = &r.channels[0];
        assert_eq!(slack.trust_tier, "core", "último registro vence o tier");
        assert_eq!(slack.install_ref, "sha-new", "último registro vence a ref");
        assert!(slack.connected, "re-registrar não desconecta");
        assert_eq!(r.expected_outbound, 1);
    }

    /// Tolerância a log degradado: `ChannelRegistered` sem `channel` é pulado (não cria canal-fantasma);
    /// sem `trust_tier`/`install_ref` cai em rótulos honestos `(sem tier)`/`(sem ref)`, nunca inventa
    /// curadoria nem panica. O `[outros]` do catálogo recolhe o tier desconhecido.
    #[test]
    fn tolerant_to_degraded_log() {
        let log = vec![
            rec(
                "ChannelRegistered",
                serde_json::json!({"trust_tier": "core"}),
            ), // sem channel → pulado
            rec(
                "ChannelRegistered",
                serde_json::json!({"channel": "misterio"}), // sem tier/ref
            ),
        ];
        let r = network_exposure(&log);
        assert_eq!(r.channels_declared, 1, "só o canal com nome conta");
        let m = &r.channels[0];
        assert_eq!(m.channel, "misterio");
        assert_eq!(m.trust_tier, "(sem tier)", "não inventa curadoria");
        assert_eq!(m.install_ref, "(sem ref)");
        // tier desconhecido cai em [outros] no catálogo (não some).
        assert!(
            catalog_section(&r.channels).contains("[outros]"),
            "{}",
            catalog_section(&r.channels)
        );
    }

    /// O render carrega o veredito do gate (d) E a verificação externa — o texto é parte do contrato
    /// (a afirmação precisa circular legível, com o COMO conferir por fora).
    #[test]
    fn render_states_zero_outbound_and_external_check() {
        let out = render(&network_exposure(&[]));
        assert!(out.contains("NENHUM socket de saída"), "{out}");
        assert!(out.contains("nettop"), "{out}");
        assert!(out.contains("lsof"), "{out}");
        assert!(out.contains("CATÁLOGO DE CANAIS"), "{out}");
    }
}
