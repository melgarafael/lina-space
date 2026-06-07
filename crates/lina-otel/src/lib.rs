//! `lina-otel` — fonte OTel **OPCIONAL** da observabilidade de sessões (story **F1-1-4**).
//!
//! ## O que é (e o que NUNCA é)
//! Um collector **local** que, quando o CLI suporta (Claude Code emite OTel se
//! configurado — `capabilities.otel` do `CliProfile`, F1-1-1), recebe métricas de
//! telemetria via **OTLP/HTTP** num receiver `127.0.0.1:4318` e as normaliza no MESMO
//! schema da F1-1-2 ([`lina_session_watch::Session`]), virando a **fonte preferida de
//! custo** sobre o JSONL (13.5 item 9 — corrige o subconto). Não é dashboard, não é
//! dependência: **enhancement, não pré-requisito** (13.10 item 7 refutou "OTel é
//! pré-requisito de governança").
//!
//! ## Invariantes que mandam aqui
//! - **#2 Local-first / opt-in sinalizado:** desligado por default ([`OtelConfig::default`]
//!   tem `enabled=false`); o receiver faz bind **só em loopback** ([`ensure_loopback`]);
//!   **nada sai da máquina** e nenhuma porta escuta fora de `127.0.0.1`. Esta story é o
//!   exemplo canônico do invariante.
//! - **#3 Neutralidade multi-CLI:** quais env vars de telemetria injetar e SE injetar
//!   vêm do `CliProfile` ([`otel_env`]) — nada hardcoded por CLI no core.
//! - **#4 Projeção:** o normalizado é PROJEÇÃO (upsert na `sessions.db`); o OTel é
//!   cumulativo (cada export carrega o total → "último export vence" por sessão).
//!
//! ## Privacidade (redaction NÃO-opcional quando ligado — 13.5 item 3)
//! Extraímos só métricas numéricas + `session.id` + `model`. Qualquer string retida/
//! logada passa pelo [`Redactor`] (e-mail, caminho de home, token) — produto não-técnico
//! não pode vazar PII num log.
//!
//! ## Degradação
//! Sem OTel (default), [`otel_env`] devolve vazio e nada muda: o JSONL da F1-1-2 segue
//! sozinho. O dashboard popula os cards nos DOIS casos (critério 2 da story).

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use lina_cli_profiles::CliProfile;
use lina_session_watch::OtelCost;
use thiserror::Error;

mod receiver;
pub use receiver::{OtelReceiver, ReceiverError, Sink};

/// Endpoint default do receiver OTLP/HTTP — **loopback** (invariante #2).
pub const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:4318";

/// Intervalo de export (ms) que injetamos no CLI: REDUZIDO do default OTLP (60 s) para
/// limitar a perda de dados num encerramento abrupto (13.5 achado 1: "batch de 5s perde
/// dados"; mitigação da story: batch timeout reduzido + persist-on-receipt no receiver).
pub const METRIC_EXPORT_INTERVAL_MS: u64 = 5_000;

/// Configuração do collector. **Opt-in** (invariante #2): `enabled=false` por default —
/// nada escuta, nenhuma env de telemetria é injetada, o JSONL segue sozinho.
#[derive(Debug, Clone)]
pub struct OtelConfig {
    /// Liga o collector. Default **false** (opt-in sinalizado).
    pub enabled: bool,
    /// Endpoint do receiver (sempre loopback — validado em [`OtelConfig::validate`]).
    pub endpoint: String,
    /// Redaction de PII. **Não-opcional quando ligado** (13.5 item 3): forçada a `true`
    /// por [`OtelConfig::validate`] se `enabled`.
    pub redact_pii: bool,
    /// Intervalo de export (ms) injetado no CLI.
    pub export_interval_ms: u64,
}

impl Default for OtelConfig {
    fn default() -> Self {
        Self {
            enabled: false, // opt-in — invariante #2
            endpoint: DEFAULT_ENDPOINT.to_string(),
            redact_pii: true,
            export_interval_ms: METRIC_EXPORT_INTERVAL_MS,
        }
    }
}

impl OtelConfig {
    /// Liga o collector mantendo os defaults seguros (redaction ON, loopback).
    #[must_use]
    pub fn enabled() -> Self {
        Self {
            enabled: true,
            ..Self::default()
        }
    }

    /// Valida e ENDURECE: endpoint deve ser loopback; redaction é forçada quando ligado
    /// (não-opcional). Devolve a config saneada ou recusa um endpoint não-local.
    ///
    /// # Errors
    /// [`OtelError::NotLocal`] se o host do endpoint não é loopback.
    pub fn validate(mut self) -> Result<Self, OtelError> {
        if self.enabled {
            self.redact_pii = true; // 13.5 item 3 — não-opcional
            let host = endpoint_host(&self.endpoint);
            if !is_loopback_host(&host) {
                return Err(OtelError::NotLocal(self.endpoint.clone()));
            }
        }
        Ok(self)
    }
}

/// **Env vars de telemetria a injetar no spawn do CLI** — SÓ quando o `CliProfile`
/// declara `capabilities.otel` (invariante #3) E o collector está ligado. Sem isso
/// (default, ou CLI que não emite OTel), devolve **vazio** → degradação graciosa, o
/// JSONL segue sozinho.
///
/// As chaves base (`CLAUDE_CODE_ENABLE_TELEMETRY`, `OTEL_*`) vêm do PROFILE quando
/// declaradas em `[env]` (neutralidade); aqui só preenchemos o endpoint/intervalo do
/// collector e garantimos o opt-in de telemetria do Claude Code.
#[must_use]
pub fn otel_env(profile: &CliProfile, config: &OtelConfig) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    if !config.enabled || !profile.capabilities.has("otel") {
        return env; // desligado, ou CLI sem OTel → nada injetado (degradação)
    }
    // O que o profile declarar em `[env]` (ex.: `CLAUDE_CODE_ENABLE_TELEMETRY=1`) tem
    // precedência de NEUTRALIDADE; preenchemos o resto.
    for (k, v) in &profile.env {
        env.insert(k.clone(), v.clone());
    }
    env.entry("CLAUDE_CODE_ENABLE_TELEMETRY".to_string())
        .or_insert_with(|| "1".to_string());
    env.insert(
        "OTEL_EXPORTER_OTLP_ENDPOINT".to_string(),
        config.endpoint.clone(),
    );
    // OTLP/HTTP JSON (o que normalizamos) — não protobuf.
    env.insert(
        "OTEL_EXPORTER_OTLP_PROTOCOL".to_string(),
        "http/json".to_string(),
    );
    env.insert("OTEL_METRICS_EXPORTER".to_string(), "otlp".to_string());
    env.insert("OTEL_LOGS_EXPORTER".to_string(), "otlp".to_string());
    env.insert(
        "OTEL_METRIC_EXPORT_INTERVAL".to_string(),
        config.export_interval_ms.to_string(),
    );
    env
}

// ───────────────────────── normalização OTLP/HTTP → schema de sessão ─────────────────────────

/// Nomes de métrica do Claude Code (monitoring docs). Outros CLIs declaram os seus via
/// profile no futuro; aqui o conjunto 1ª-classe.
const METRIC_TOKEN_USAGE: &str = "claude_code.token.usage";
const METRIC_COST_USAGE: &str = "claude_code.cost.usage";

/// Chaves de atributo onde o id de sessão pode aparecer (resource OU datapoint).
const SESSION_ID_KEYS: &[&str] = &["session.id", "session_id", "user.session.id"];

/// **Normaliza um export OTLP/HTTP-JSON de métricas** em `OtelCost` por sessão.
/// Counters OTLP são CUMULATIVOS — o valor de cada ponto é o total da sessão; o receiver
/// faz upsert (último export vence). Tolerante: campos ausentes/extras são ignorados,
/// nunca panica. As strings retidas (model/session) passam pelo [`Redactor`] quando dado.
#[must_use]
pub fn normalize_metrics(
    export: &serde_json::Value,
    redactor: Option<&Redactor>,
) -> BTreeMap<String, OtelCost> {
    let mut out: BTreeMap<String, OtelCost> = BTreeMap::new();
    let resource_metrics = export
        .get("resourceMetrics")
        .and_then(|v| v.as_array())
        .map(Vec::as_slice)
        .unwrap_or_default();
    for rm in resource_metrics {
        let resource_attrs = attr_map(rm.get("resource").and_then(|r| r.get("attributes")));
        let scope_metrics = rm
            .get("scopeMetrics")
            .and_then(|v| v.as_array())
            .map(Vec::as_slice)
            .unwrap_or_default();
        for sm in scope_metrics {
            let metrics = sm
                .get("metrics")
                .and_then(|v| v.as_array())
                .map(Vec::as_slice)
                .unwrap_or_default();
            for metric in metrics {
                let name = metric.get("name").and_then(|v| v.as_str()).unwrap_or("");
                if name != METRIC_TOKEN_USAGE && name != METRIC_COST_USAGE {
                    continue;
                }
                let points = metric
                    .get("sum")
                    .and_then(|s| s.get("dataPoints"))
                    .and_then(|v| v.as_array())
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                for dp in points {
                    let dp_attrs = attr_map(dp.get("attributes"));
                    let session = session_id_of(&resource_attrs, &dp_attrs);
                    let Some(session) = session else { continue };
                    let session = redactor.map_or_else(|| session.clone(), |r| r.redact(&session));
                    let entry = out.entry(session).or_default();
                    let value = point_value(dp);
                    if let Some(model) = dp_attrs
                        .get("model")
                        .or_else(|| resource_attrs.get("model"))
                    {
                        let model = redactor.map_or_else(|| model.clone(), |r| r.redact(model));
                        entry.model = Some(model);
                    }
                    if name == METRIC_COST_USAGE {
                        entry.cost_usd = Some(entry.cost_usd.unwrap_or(0.0) + value);
                    } else {
                        // token.usage por `type` (input/output/cacheRead/cacheCreation).
                        match dp_attrs.get("type").map(String::as_str) {
                            Some("input") => entry.tokens_in = value as u64,
                            Some("output") => entry.tokens_out = value as u64,
                            Some("cacheRead" | "cacheCreation") => {
                                entry.tokens_cache += value as u64;
                            }
                            Some("thinking") => entry.tokens_thinking = value as u64,
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    out
}

/// Valor numérico de um datapoint OTLP (`asInt` como string/num, ou `asDouble`).
fn point_value(dp: &serde_json::Value) -> f64 {
    if let Some(i) = dp.get("asInt") {
        // OTLP/JSON codifica int64 como STRING (spec); aceita number também (tolerante).
        if let Some(s) = i.as_str() {
            return s.parse::<f64>().unwrap_or(0.0);
        }
        if let Some(n) = i.as_f64() {
            return n;
        }
    }
    dp.get("asDouble")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0)
}

/// Mapa `key → stringValue` de uma lista de atributos OTLP (`[{key, value:{stringValue}}]`).
fn attr_map(attrs: Option<&serde_json::Value>) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    let Some(arr) = attrs.and_then(|v| v.as_array()) else {
        return m;
    };
    for a in arr {
        let Some(key) = a.get("key").and_then(|v| v.as_str()) else {
            continue;
        };
        let val = a.get("value");
        let s = val
            .and_then(|v| v.get("stringValue"))
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned)
            .or_else(|| {
                val.and_then(|v| v.get("intValue"))
                    .and_then(|v| v.as_str())
                    .map(ToOwned::to_owned)
            });
        if let Some(s) = s {
            m.insert(key.to_owned(), s);
        }
    }
    m
}

fn session_id_of(
    resource: &BTreeMap<String, String>,
    dp: &BTreeMap<String, String>,
) -> Option<String> {
    for key in SESSION_ID_KEYS {
        if let Some(v) = dp.get(*key).or_else(|| resource.get(*key)) {
            return Some(v.clone());
        }
    }
    None
}

// ───────────────────────── redaction de PII ─────────────────────────

/// **Processor de redaction de PII** — não-opcional quando o collector está ligado
/// (13.5 item 3). Mascara, sem regex pesada: e-mails, segmentos de home (`/Users/<x>`,
/// `/home/<x>`, `C:\Users\<x>`) e tokens (`sk-…`, `Bearer …`, `ghp_…`). Determinístico.
#[derive(Debug, Default, Clone)]
pub struct Redactor;

impl Redactor {
    /// Mascara PII de uma string (idempotente). Preserva o formato o suficiente para o
    /// dado seguir útil (ex.: `/Users/⟨user⟩/proj`).
    #[must_use]
    pub fn redact(&self, s: &str) -> String {
        let mut out = redact_emails(s);
        out = redact_home_paths(&out);
        out = redact_tokens(&out);
        out
    }
}

fn redact_emails(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for token in split_keep(s) {
        if token.contains('@') && token.contains('.') && !token.contains(' ') {
            // local@dom.tld → ⟨email⟩
            let at = token.find('@').unwrap_or(0);
            let dot = token[at..].contains('.');
            if at > 0 && dot {
                out.push_str("⟨email⟩");
                continue;
            }
        }
        out.push_str(token);
    }
    out
}

fn redact_home_paths(s: &str) -> String {
    // Substitui o COMPONENTE de usuário após /Users//home//root marcadores.
    let mut result = s.to_string();
    for marker in ["/Users/", "/home/", "\\Users\\"] {
        result = replace_user_component(&result, marker);
    }
    result
}

fn replace_user_component(s: &str, marker: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find(marker) {
        out.push_str(&rest[..pos + marker.len()]);
        rest = &rest[pos + marker.len()..];
        // o componente de usuário vai até o próximo separador.
        let end = rest.find(['/', '\\', ' ', '"', '\'']).unwrap_or(rest.len());
        if end > 0 {
            out.push_str("⟨user⟩");
            rest = &rest[end..];
        }
    }
    out.push_str(rest);
    out
}

fn redact_tokens(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    // `Bearer X` / `bearer X`: o segredo é a PRÓXIMA palavra → redige-a quando aparecer.
    let mut redact_next_word = false;
    for token in split_keep(s) {
        let t = token.trim();
        if redact_next_word && !t.is_empty() {
            out.push_str("⟨token⟩");
            redact_next_word = false;
            continue;
        }
        let standalone_secret =
            (t.starts_with("sk-") && t.len() > 8) || (t.starts_with("ghp_") && t.len() > 8);
        if standalone_secret {
            out.push_str("⟨token⟩");
        } else {
            if t.eq_ignore_ascii_case("Bearer") {
                redact_next_word = true; // a próxima palavra (não-separador) é o segredo
            }
            out.push_str(token);
        }
    }
    out
}

/// Tokeniza preservando os separadores (espaço/aspas) para reconstrução fiel.
fn split_keep(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    for (i, c) in s.char_indices() {
        if c == ' ' || c == '"' || c == '\'' || c == ',' {
            if i > start {
                parts.push(&s[start..i]);
            }
            parts.push(&s[i..i + c.len_utf8()]);
            start = i + c.len_utf8();
        }
    }
    if start < s.len() {
        parts.push(&s[start..]);
    }
    parts
}

// ───────────────────────── sanity check JSONL × OTel ─────────────────────────

/// Resultado do merge OTel→sessão para o chamador (com a divergência do sanity check).
#[derive(Debug, Clone)]
pub struct Merged {
    pub session: lina_session_watch::Session,
    pub divergence: f64,
    pub diverged: bool,
}

/// **Funde o `OtelCost` na sessão JSONL** (OTel = fonte preferida) e roda o sanity
/// check de divergência de custo (13.5 item 1: LOGA quando discordam acima do threshold).
/// O log é via `tracing::warn` (visível, não-fatal) — a observabilidade não quebra.
#[must_use]
pub fn merge_and_check(jsonl: &lina_session_watch::Session, otel: &OtelCost) -> Merged {
    let (session, divergence) = jsonl.merge_otel_cost(otel);
    let diverged = divergence > lina_session_watch::COST_DIVERGENCE_THRESHOLD;
    if diverged {
        tracing::warn!(
            cli = %session.cli,
            session = %session.session_id,
            jsonl_cost = jsonl.cost_usd,
            otel_cost = otel.cost_usd.unwrap_or(jsonl.cost_usd),
            divergence,
            "sanity check JSONL×OTel: custo diverge acima do threshold (13.5 item 1)"
        );
    }
    Merged {
        session,
        divergence,
        diverged,
    }
}

// ───────────────────────── loopback guards ─────────────────────────

/// Host de uma URL `scheme://host:port` (sem dep de crate de URL — basta para validar).
#[must_use]
pub fn endpoint_host(endpoint: &str) -> String {
    let after_scheme = endpoint.split("://").nth(1).unwrap_or(endpoint);
    let host_port = after_scheme.split('/').next().unwrap_or(after_scheme);
    host_port
        .rsplit_once(':')
        .map_or(host_port, |(h, _)| h)
        .to_string()
}

/// `true` se o host é uma interface loopback (`127.x`, `::1`, `localhost`).
#[must_use]
pub fn is_loopback_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

/// Guard do invariante #2 para um `SocketAddr` já resolvido (o receiver recusa servir
/// fora de loopback — nunca confia cegamente no chamador).
///
/// # Errors
/// [`OtelError::NotLocal`] se o IP não é loopback.
pub fn ensure_loopback(addr: SocketAddr) -> Result<(), OtelError> {
    if addr.ip().is_loopback() {
        Ok(())
    } else {
        Err(OtelError::NotLocal(addr.to_string()))
    }
}

// ───────────────────────── sink compartilhado (persist-on-receipt) ─────────────────────────

/// Coletor de sessões normalizadas em memória — útil como sink de teste e como base do
/// upsert na `SessionProjection` (persist-on-receipt: o receiver chama isto a cada
/// export, SEM janela de batch própria — a mitigação do critério 3).
#[derive(Debug, Clone, Default)]
pub struct CostStore(Arc<Mutex<BTreeMap<String, OtelCost>>>);

impl CostStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    /// Upsert cumulativo por sessão (counters OTLP são totais → último export vence).
    pub fn upsert(&self, session_id: &str, cost: OtelCost) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(session_id.to_owned(), cost);
    }
    #[must_use]
    pub fn get(&self, session_id: &str) -> Option<OtelCost> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(session_id)
            .cloned()
    }
    #[must_use]
    pub fn len(&self) -> usize {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Erros do collector — acionáveis, nunca panic.
#[derive(Debug, Error)]
pub enum OtelError {
    /// Endpoint/listener fora de loopback (invariante #2 local-first).
    #[error("endpoint/bind não-local recusado (local-first; use 127.0.0.1): {0}")]
    NotLocal(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use lina_session_watch::{CostSource, Session};

    fn jsonl_session(cost: f64) -> Session {
        Session {
            cli: "claude-code".into(),
            session_id: "sess-1".into(),
            tokens_in: 100,
            tokens_out: 50,
            tokens_cache: 0,
            tokens_thinking: 0,
            cost_usd: cost,
            cost_estimated: true,
            model: Some("claude".into()),
            cwd: Some("/Users/rafa/proj".into()),
            last_ts: Some("2026-06-06T00:00:00Z".into()),
            subagents: vec![],
            tools: vec!["Bash".into()],
            source: CostSource::Jsonl,
        }
    }

    fn metrics_export(session: &str) -> serde_json::Value {
        serde_json::json!({
            "resourceMetrics": [{
                "resource": { "attributes": [
                    {"key": "session.id", "value": {"stringValue": session}}
                ]},
                "scopeMetrics": [{
                    "metrics": [
                        {"name": "claude_code.token.usage", "sum": {"dataPoints": [
                            {"attributes": [{"key":"type","value":{"stringValue":"input"}},
                                            {"key":"model","value":{"stringValue":"claude-opus"}}],
                             "asInt": "17400"},
                            {"attributes": [{"key":"type","value":{"stringValue":"output"}}],
                             "asInt": "820"},
                            {"attributes": [{"key":"type","value":{"stringValue":"cacheRead"}}],
                             "asInt": "5000"}
                        ]}},
                        {"name": "claude_code.cost.usage", "sum": {"dataPoints": [
                            {"attributes": [], "asDouble": 0.42}
                        ]}}
                    ]
                }]
            }]
        })
    }

    #[test]
    fn opt_in_default_off_e_loopback() {
        let c = OtelConfig::default();
        assert!(!c.enabled, "default DESLIGADO (opt-in — invariante #2)");
        assert_eq!(endpoint_host(DEFAULT_ENDPOINT), "127.0.0.1");
        assert!(
            is_loopback_host("127.0.0.1")
                && is_loopback_host("localhost")
                && is_loopback_host("::1")
        );
        assert!(!is_loopback_host("0.0.0.0") && !is_loopback_host("10.0.0.5"));
        // Endpoint não-local é recusado quando ligado.
        let bad = OtelConfig {
            enabled: true,
            endpoint: "http://10.0.0.5:4318".into(),
            ..OtelConfig::default()
        };
        assert!(matches!(bad.validate(), Err(OtelError::NotLocal(_))));
        // Ligar força redaction (não-opcional).
        let ok = OtelConfig {
            enabled: true,
            redact_pii: false,
            ..OtelConfig::default()
        }
        .validate()
        .expect("loopback ok");
        assert!(
            ok.redact_pii,
            "redaction é forçada quando ligado (13.5 item 3)"
        );
    }

    #[test]
    fn env_so_quando_capability_e_ligado() {
        let toml = r#"
            id = "claude-code"
            program = "claude"
            delivery = "pty_inject"
            submit_delay_ms = 300
            prompt_ready_regex = '>'
            [end_signal]
            kind = "idle"
            [capabilities]
            otel = true
        "#;
        let prof = CliProfile::from_toml_str(toml, "<t>").expect("profile");
        // Desligado → vazio (degradação).
        assert!(otel_env(&prof, &OtelConfig::default()).is_empty());
        // Ligado + capability → env de telemetria preenchido.
        let env = otel_env(&prof, &OtelConfig::enabled());
        assert_eq!(
            env.get("CLAUDE_CODE_ENABLE_TELEMETRY").map(String::as_str),
            Some("1")
        );
        assert_eq!(
            env.get("OTEL_EXPORTER_OTLP_ENDPOINT").map(String::as_str),
            Some(DEFAULT_ENDPOINT)
        );
        assert_eq!(
            env.get("OTEL_EXPORTER_OTLP_PROTOCOL").map(String::as_str),
            Some("http/json")
        );

        // CLI SEM otel → vazio mesmo ligado (degradação; nada injetado).
        let toml2 = r#"
            id = "codex"
            program = "codex"
            delivery = "pty_inject"
            submit_delay_ms = 300
            prompt_ready_regex = '>'
            [end_signal]
            kind = "idle"
            [capabilities]
            otel = false
        "#;
        let codex = CliProfile::from_toml_str(toml2, "<t>").expect("profile");
        assert!(otel_env(&codex, &OtelConfig::enabled()).is_empty());
    }

    #[test]
    fn normaliza_export_e_funde_como_fonte_preferida() {
        let costs = normalize_metrics(&metrics_export("sess-1"), None);
        let otel = costs.get("sess-1").expect("sessão normalizada");
        assert_eq!(otel.tokens_in, 17_400);
        assert_eq!(otel.tokens_out, 820);
        assert_eq!(otel.tokens_cache, 5_000);
        assert_eq!(otel.cost_usd, Some(0.42));
        assert_eq!(otel.model.as_deref(), Some("claude-opus"));

        // Merge: OTel vira a fonte preferida; custo deixa de ser estimativa.
        let jsonl = jsonl_session(0.40);
        let merged = merge_and_check(&jsonl, otel);
        assert_eq!(
            merged.session.source,
            CostSource::Otel,
            "fonte vira OTel (critério 1)"
        );
        assert!(
            !merged.session.cost_estimated,
            "custo OTEL medido não é estimativa"
        );
        assert_eq!(merged.session.cost_usd, 0.42);
        assert_eq!(
            merged.session.tokens_in, 17_400,
            "tokens do OTel corrigem o subconto"
        );
        // cwd/tools (não-custo) seguem do JSONL.
        assert_eq!(merged.session.cwd.as_deref(), Some("/Users/rafa/proj"));
        assert_eq!(merged.session.tools, vec!["Bash".to_string()]);
        // Sanity: 0.40 vs 0.42 → ~4.8% < 20% → não diverge.
        assert!(
            !merged.diverged,
            "divergência {} dentro do threshold",
            merged.divergence
        );

        // Divergência grande → sinalizada.
        let big = merge_and_check(&jsonl_session(0.10), otel);
        assert!(big.diverged, "0.10 vs 0.42 diverge (>20%)");
    }

    #[test]
    fn so_tokens_sem_custo_deriva_estimativa() {
        // Export só com token.usage (Gemini/Antigravity: tokens, custo derivado).
        let export = serde_json::json!({
            "resourceMetrics": [{
                "resource": {"attributes": [{"key":"session.id","value":{"stringValue":"s2"}}]},
                "scopeMetrics": [{
                    "metrics": [
                        {"name":"claude_code.token.usage","sum":{"dataPoints":[
                            {"attributes":[{"key":"type","value":{"stringValue":"input"}}],"asInt":"10"}
                        ]}}
                    ]
                }]
            }]
        });
        let costs = normalize_metrics(&export, None);
        let otel = costs.get("s2").expect("s2");
        assert_eq!(otel.cost_usd, None, "sem cost.usage → cost None");
        let merged = merge_and_check(&jsonl_session(0.5), otel);
        assert!(
            merged.session.cost_estimated,
            "só-tokens → custo segue estimativa (derivado), mesmo via OTel"
        );
        assert_eq!(merged.session.source, CostSource::Otel);
    }

    #[test]
    fn redaction_mascara_pii() {
        let r = Redactor;
        assert_eq!(r.redact("user rafa@maud.com.br aqui"), "user ⟨email⟩ aqui");
        assert!(r
            .redact("/Users/rafael/proj/x")
            .starts_with("/Users/⟨user⟩/"));
        assert!(r.redact("/home/melga/code").contains("/home/⟨user⟩"));
        assert_eq!(r.redact("token sk-abcdef123456 fim"), "token ⟨token⟩ fim");
        assert!(r.redact("Bearer abcdefghij").contains("⟨token⟩"));
        // Texto inócuo intacto.
        assert_eq!(r.redact("claude-opus-4"), "claude-opus-4");
    }

    #[test]
    fn normalize_com_redactor_mascara_model_e_session() {
        let export = serde_json::json!({
            "resourceMetrics": [{
                "resource": {"attributes": [{"key":"session.id","value":{"stringValue":"/Users/rafa/s"}}]},
                "scopeMetrics": [{
                    "metrics": [
                        {"name":"claude_code.token.usage","sum":{"dataPoints":[
                            {"attributes":[{"key":"type","value":{"stringValue":"input"}},
                                           {"key":"model","value":{"stringValue":"sk-leak123456"}}],"asInt":"1"}
                        ]}}
                    ]
                }]
            }]
        });
        let costs = normalize_metrics(&export, Some(&Redactor));
        // a chave de sessão foi redigida → procura pela forma mascarada.
        let (_k, v) = costs.iter().next().expect("uma sessão");
        assert_eq!(
            v.model.as_deref(),
            Some("⟨token⟩"),
            "model com PII é mascarado"
        );
        assert!(
            costs.keys().any(|k| k.contains("⟨user⟩")),
            "session id com path é redigido"
        );
    }

    #[test]
    fn export_vazio_ou_lixo_nao_panica() {
        assert!(normalize_metrics(&serde_json::json!({}), None).is_empty());
        assert!(
            normalize_metrics(&serde_json::json!({"resourceMetrics": "lixo"}), None).is_empty()
        );
        assert!(normalize_metrics(&serde_json::json!(42), None).is_empty());
    }
}
