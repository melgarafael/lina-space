//! [12] Parâmetros versionados do sistema (spec 51; ponto [12] de Meadows — o mais fraco da
//! escala, que só ganha sentido amarrado aos pontos altos que empurra: [7] freio do laço de
//! delegação, [8] ganho dos breakers, [9] delay-de-raciocínio do effort).
//!
//! Hoje os ~15 números que governam a orquestração vivem como `pub const` + [`RouterConfig`]
//! (`router.rs`), e 13 deles caem no `..RouterConfig::default()` de `bridge.rs:717` — um knob
//! morto que parece configurável. Este módulo dá-lhes **casa estruturada e versionada**:
//!
//! - **`None` num campo = "esta camada não opina; herda da camada de baixo".** É o que permite
//!   fundir camadas sem que uma superior precise re-declarar tudo.
//! - **ADITIVO por construção** (todo campo `Option<T>` + `#[serde(default)]`): carregar um
//!   `params.json` antigo, ou replayar um log antigo, nunca quebra — espelha o padrão de
//!   `events.rs` e de `submit_delay_ms` (`lina-cli-profiles`).
//! - **Resolução determinística em 4 camadas** (`global ⊕ workspace ⊕ preset ⊕ terminal`,
//!   precedência crescente — reusa a doutrina "instrução-corrente > plano > vault" do projeto)
//!   produz um [`RouterConfig`] denso + os campos que ele ainda não tem (`effort`, `model`,
//!   `max_cycle_revisits`, `scrollback_retention_days`), aplicando default + clamp de faixa.
//! - **Faixa enforçada SEM panic** (spec 51 §Segurança/critério 3): valor fora da faixa é
//!   clampado e gera um [`ParamWarning`] (a fila de atenção o consome em F3-0-2/0-7); o processo
//!   sobe e roteia. Parâmetro muda *magnitude*, jamais *autoridade* (ADR 0007 é o guardião).
//!
//! Esta story (F3-0-1) é a **casa de tipos** — a fiação (`bridge.rs`/`router.rs` virarem
//! projeção do log via `resolve_from_store`) é F3-0-2; `model`/`effort` no envelope de spawn é
//! F3-0-3. Aqui nada toca `router.rs`/`bridge.rs`.

use serde::{Deserialize, Serialize};

use crate::events::{Effort, EventRecord, EventStore, StoreError};
use crate::guard::ActionClass;
use crate::lifecycle::{HEARTBEAT_SAMPLE_MS, STALL_BREAKER_SAMPLES, STALL_WARN_SAMPLES};
use crate::router::{
    AutonomyLevel, RouterConfig, AWAIT_TIMEOUT_MS, BREAKER_THRESHOLD, DEDUPE_WINDOW_MS,
    DELEGATION_BUDGET, DELIVERY_BACKOFF_BASE_MS, DELIVERY_MAX_ATTEMPTS, FANOUT_GATE, MAX_CYCLE_REVISITS,
    MAX_DEPTH, MAX_SPAWNS_PER_TURN, RETENTION_TIMEOUT_MS,
};
use crate::scrollback::DEFAULT_RETENTION_DAYS;

/// A camada que aplicou um parâmetro, em precedência **crescente** (`Terminal` vence todas).
/// Serializa em `lowercase` para casar o `scope` de `SystemParamsChanged` e o `--scope` do CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParamScope {
    /// `~/.lina/params.json` — menor precedência.
    Global,
    /// `.lina/params.json` do workspace.
    Workspace,
    /// Pacote carregado por um `FocusPreset` (o veículo de calibração do leigo).
    Preset,
    /// Camada do envelope de spawn / atribuição do Maestro — maior precedência.
    Terminal,
}

/// Um valor que veio fora da faixa válida e foi **clampado** no load/resolve (spec 51 §4: WARN,
/// nunca panic). Estruturado (não texto livre) para a fila de atenção formatar ao leigo em pt-br
/// e para os testes asserterem. `requested`/`clamped` são serializados como string — auditável
/// sem schema rígido, igual ao `old`/`new` de `SystemParamsChanged`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParamWarning {
    /// Chave canônica do parâmetro (ex.: `"delegation_budget"`).
    pub key: String,
    /// Valor pedido, fora da faixa (serializado).
    pub requested: String,
    /// Valor efetivo após o clamp (serializado).
    pub clamped: String,
}

/// Uma camada de parâmetros. Toda chave é `Option`: `None` = herda da camada de baixo; `Some` =
/// sobrescreve. **ADITIVO**: campos futuros (knobs de browser-portal, bus cross-machine — ponto
/// [10]) entram aqui com `#[serde(default)]` sem migração. Os comentários fixam, por campo, a
/// origem (`router.rs`/`lifecycle.rs`/`scrollback.rs`), o default e a faixa válida (spec 51 §1).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SystemParams {
    // ── Anti-loop / orçamento (freio do laço [7]; ganho dos laços [8]) ──
    /// `router.rs` default 8 · faixa `1..=64`.
    pub delegation_budget: Option<u32>,
    /// `router.rs` default 4 · faixa `1..=8`.
    pub max_depth: Option<u8>,
    /// `router.rs` default 3 · faixa `1..=16`.
    pub fanout_gate: Option<usize>,
    /// `router.rs` default 2 · faixa `0..=8`.
    pub max_spawns_per_turn: Option<u32>,
    /// `router.rs` default 2 · faixa `1..=8`.
    pub max_cycle_revisits: Option<u32>,
    // ── Defasagens de entrega/retry/espera (ponto [9]) ──
    /// `router.rs` default 60_000 · faixa `1_000..=600_000`.
    pub dedupe_window_ms: Option<u64>,
    /// `router.rs` default 300_000 · faixa `30_000..=1_800_000`.
    pub await_timeout_ms: Option<u64>,
    /// `router.rs` default 600_000 · faixa `30_000..=3_600_000`.
    pub retention_timeout_ms: Option<u64>,
    /// `router.rs` default 5 · faixa `1..=20`.
    pub delivery_max_attempts: Option<u32>,
    /// `router.rs` default 1_000 · faixa `100..=60_000`.
    pub delivery_backoff_base_ms: Option<u64>,
    /// `router.rs` default 5 · faixa `1..=20`.
    pub breaker_threshold: Option<u32>,
    // ── Detecção de travamento (lifecycle.rs; pontos [8]/[9]) ──
    /// `lifecycle.rs` default 120_000 · faixa `30_000..=600_000`.
    pub heartbeat_sample_ms: Option<u64>,
    /// `lifecycle.rs` default 3 · faixa `1..=20`.
    pub stall_warn_samples: Option<u32>,
    /// `lifecycle.rs` default 6 · faixa `2..=40`.
    pub stall_breaker_samples: Option<u32>,
    // ── Custo / retenção visíveis ao leigo (via preset) ──
    /// `router.rs` default 0 (desligado, opt-in) · sem faixa (qualquer `u64`).
    pub token_budget_day: Option<u64>,
    /// `scrollback.rs` default 30 · faixa `0..=365` (0 = desligado).
    pub scrollback_retention_days: Option<u32>,
    // ── NOVO: cognição por terminal/papel/preset (linha 57/58 do doc-fonte) ──
    /// Modelo herdado do CLI Profile (alias concreto é do profile — invariante #3). `None` = o
    /// que o CLI já usa. A *seleção* mora aqui; o *vocabulário* é do profile.
    pub model: Option<String>,
    /// Nível de raciocínio · default [`Effort::Medium`] · contrato neutro `low/medium/high`.
    pub effort: Option<Effort>,
}

/// O resultado da resolução das 4 camadas: o [`RouterConfig`] denso (os campos que o router já
/// tem) **mais** os campos que ele ainda não tem (`effort`/`model`/`max_cycle_revisits`/
/// `scrollback_retention_days` — fiados em F3-0-2/0-3 sem tocar `router.rs` agora) e os
/// [`ParamWarning`]s coletados no clamp. É o tipo que `resolve_from_store` (F3-0-2) devolve — daí
/// o campo nomeado `router_config` (gate de saída (b): `…router_config.fanout_gate == 8`).
#[derive(Debug, Clone)]
pub struct ResolvedParams {
    /// Os 14 campos tunáveis + `autonomy` (do bootstrap), prontos para o [`Router`](crate::Router).
    pub router_config: RouterConfig,
    /// Nível de raciocínio efetivo (default [`Effort::Medium`]).
    pub effort: Effort,
    /// Modelo selecionado, se algum (`None` = o que o CLI já usa).
    pub model: Option<String>,
    /// Retenção de scrollback efetiva (dias; alimenta `ScrollbackConfig`, F1-5-9).
    pub scrollback_retention_days: u32,
    /// Teto de revisitas de ciclo efetivo (hoje `MAX_CYCLE_REVISITS` é const no router; vira
    /// campo do `RouterConfig` quando F3-0-2 fiar a projeção).
    pub max_cycle_revisits: u32,
    /// Valores que vieram fora da faixa e foram clampados (a fila de atenção os consome).
    pub warnings: Vec<ParamWarning>,
}

/// Faixa válida `[min, max]` de um parâmetro NUMÉRICO, por chave canônica (spec 51 §1). **FONTE
/// ÚNICA** das 16 faixas: tanto o clamp do [`SystemParams::into_resolved`] quanto o
/// [`validate_range`] do `lina params set` a consultam — o bin NUNCA reimplementa as faixas (COD-5).
/// `None` = chave sem faixa numérica (`model`/`effort`) ou desconhecida.
fn param_range(key: &str) -> Option<(u64, u64)> {
    let range = match key {
        "delegation_budget" => (1, 64),
        "max_depth" => (1, 8),
        "fanout_gate" => (1, 16),
        "max_spawns_per_turn" => (0, 8),
        "max_cycle_revisits" => (1, 8),
        "dedupe_window_ms" => (1_000, 600_000),
        "await_timeout_ms" => (30_000, 1_800_000),
        "retention_timeout_ms" => (30_000, 3_600_000),
        "delivery_max_attempts" => (1, 20),
        "delivery_backoff_base_ms" => (100, 60_000),
        "breaker_threshold" => (1, 20),
        "heartbeat_sample_ms" => (30_000, 600_000),
        "stall_warn_samples" => (1, 20),
        "stall_breaker_samples" => (2, 40),
        "token_budget_day" => (0, u64::MAX), // só "desligado(0)..ilimitado" — sem teto artificial
        "scrollback_retention_days" => (0, 365),
        _ => return None,
    };
    Some(range)
}

/// Aplica a um campo `Option<T>` a regra default-ou-clamp consultando a faixa de `key` em
/// [`param_range`] (fonte única). `None` cai no `default`; `Some(v)` fora da faixa é clampado à
/// borda + registra [`ParamWarning`]. Nunca entra em pânico (postura best-effort, spec 51 §4). O
/// valor clampado cabe no tipo de destino por construção (a faixa ⊆ domínio de `T`); a passagem por
/// `u64` é o que dá a fonte única, e `try_from`/`unwrap_or(default)` é a rede defensiva (COD-4).
fn clamp_keyed<T>(opt: Option<T>, default: T, key: &str, warnings: &mut Vec<ParamWarning>) -> T
where
    T: Copy + ToString + TryFrom<u64>,
    u64: TryFrom<T>,
{
    let Some(v) = opt else { return default };
    let Ok(raw) = u64::try_from(v) else { return v };
    let (min, max) = param_range(key).unwrap_or((u64::MIN, u64::MAX));
    let clamped = raw.clamp(min, max);
    if clamped == raw {
        return v;
    }
    warnings.push(ParamWarning {
        key: key.to_string(),
        requested: v.to_string(),
        clamped: clamped.to_string(),
    });
    T::try_from(clamped).unwrap_or(default)
}

/// **F3-0-5: a validação de faixa que o `lina params set` consulta** — o expert recebe a faixa na
/// mensagem em vez de um clamp silencioso (spec 51 §Superfície: o `set` RECUSA, o load clampa). O
/// core é a fonte única; o bin não reimplementa as 16 faixas (COD-5). `effort`/`model` têm
/// vocabulário próprio; chave desconhecida ou valor não-numérico são recusados.
///
/// # Errors
/// Mensagem pt-br nomeando o problema e a faixa/vocabulário válido.
pub fn validate_range(key: &str, value: &str) -> Result<(), String> {
    match key {
        "effort" => match value {
            "low" | "medium" | "high" => Ok(()),
            _ => Err(format!("effort inválido {value:?}: use low, medium ou high")),
        },
        // O vocabulário concreto de modelo é do CLI Profile (invariante #3) — aqui aceita o alias.
        "model" => Ok(()),
        _ => {
            let Some((min, max)) = param_range(key) else {
                return Err(format!("parâmetro desconhecido: {key:?}"));
            };
            let n: u64 = value
                .parse()
                .map_err(|_| format!("{key} espera um número inteiro, veio {value:?}"))?;
            if n < min || n > max {
                Err(format!("{key}={n} fora da faixa válida {min}..={max}"))
            } else {
                Ok(())
            }
        }
    }
}

/// **F3-0-7 §C: classifica uma mudança de parâmetro pela DEFESA que ela toca** (spec 51 §Seg 3).
/// REDUZIR um dos LAÇOS BALANCEADORES (`delegation_budget`, `fanout_gate`, `max_depth`,
/// `breaker_threshold`, `stall_breaker_samples`) abaixo do seu default ENFRAQUECE uma defesa do
/// sistema → [`ActionClass::GatedHard`] (sobe ao gate humano, MESMO em autônomo — `guard::decide`).
/// Aumentar/fortalecer, ou mexer num parâmetro NÃO-defensivo (custo/retenção têm seus próprios
/// gates — ADR 0005), é [`ActionClass::Routine`] (aplica livre, sujeito à faixa). É o "não desligue
/// o termostato que nunca dispara" de Meadows: mexer no ganho de um laço balanceador não é rotina.
#[must_use]
pub fn param_change_action_class(key: &str, new: u64) -> ActionClass {
    let default = match key {
        "delegation_budget" => u64::from(DELEGATION_BUDGET),
        "fanout_gate" => FANOUT_GATE as u64,
        "max_depth" => u64::from(MAX_DEPTH),
        "breaker_threshold" => u64::from(BREAKER_THRESHOLD),
        "stall_breaker_samples" => u64::from(STALL_BREAKER_SAMPLES),
        _ => return ActionClass::Routine, // não-defensivo: livre (sujeito à faixa)
    };
    if new < default {
        ActionClass::GatedHard
    } else {
        ActionClass::Routine
    }
}

impl SystemParams {
    /// Funde `over` (maior precedência) sobre `self` (base): cada campo `Some` de `over` vence;
    /// `None` herda de `self`. ADITIVO por construção (um campo novo só precisa de mais uma linha
    /// `.or(...)`). É a primitiva da resolução em camadas.
    #[must_use]
    pub fn overlay(self, over: SystemParams) -> SystemParams {
        SystemParams {
            delegation_budget: over.delegation_budget.or(self.delegation_budget),
            max_depth: over.max_depth.or(self.max_depth),
            fanout_gate: over.fanout_gate.or(self.fanout_gate),
            max_spawns_per_turn: over.max_spawns_per_turn.or(self.max_spawns_per_turn),
            max_cycle_revisits: over.max_cycle_revisits.or(self.max_cycle_revisits),
            dedupe_window_ms: over.dedupe_window_ms.or(self.dedupe_window_ms),
            await_timeout_ms: over.await_timeout_ms.or(self.await_timeout_ms),
            retention_timeout_ms: over.retention_timeout_ms.or(self.retention_timeout_ms),
            delivery_max_attempts: over.delivery_max_attempts.or(self.delivery_max_attempts),
            delivery_backoff_base_ms: over.delivery_backoff_base_ms.or(self.delivery_backoff_base_ms),
            breaker_threshold: over.breaker_threshold.or(self.breaker_threshold),
            heartbeat_sample_ms: over.heartbeat_sample_ms.or(self.heartbeat_sample_ms),
            stall_warn_samples: over.stall_warn_samples.or(self.stall_warn_samples),
            stall_breaker_samples: over.stall_breaker_samples.or(self.stall_breaker_samples),
            token_budget_day: over.token_budget_day.or(self.token_budget_day),
            scrollback_retention_days: over.scrollback_retention_days.or(self.scrollback_retention_days),
            model: over.model.or(self.model),
            effort: over.effort.or(self.effort),
        }
    }

    /// Resolve as 4 camadas (`global ⊕ workspace ⊕ preset ⊕ terminal`, precedência crescente) e
    /// produz um [`ResolvedParams`], aplicando default + clamp de faixa (WARN, nunca panic).
    /// `autonomy` continua vindo do bootstrap (`bridge.rs`), não de `params.json`.
    #[must_use]
    pub fn resolve(
        global: SystemParams,
        workspace: SystemParams,
        preset: SystemParams,
        terminal: SystemParams,
        autonomy: AutonomyLevel,
    ) -> ResolvedParams {
        global
            .overlay(workspace)
            .overlay(preset)
            .overlay(terminal)
            .into_resolved(autonomy)
    }

    /// Colapsa esta camada (já fundida) num [`ResolvedParams`] denso: cada campo cai no default ou
    /// é clampado à sua faixa; `autonomy` vem de fora. É o ponto único onde defaults e faixas da
    /// spec 51 §1 são aplicados — `resolve` e o futuro `resolve_from_store` (F3-0-2) o reusam.
    #[must_use]
    pub fn into_resolved(self, autonomy: AutonomyLevel) -> ResolvedParams {
        let mut warnings = Vec::new();
        let router_config = RouterConfig {
            dedupe_window_ms: clamp_keyed(
                self.dedupe_window_ms, DEDUPE_WINDOW_MS, "dedupe_window_ms", &mut warnings,
            ),
            max_depth: clamp_keyed(self.max_depth, MAX_DEPTH, "max_depth", &mut warnings),
            delegation_budget: clamp_keyed(
                self.delegation_budget, DELEGATION_BUDGET, "delegation_budget", &mut warnings,
            ),
            max_spawns_per_turn: clamp_keyed(
                self.max_spawns_per_turn, MAX_SPAWNS_PER_TURN, "max_spawns_per_turn", &mut warnings,
            ),
            fanout_gate: clamp_keyed(self.fanout_gate, FANOUT_GATE, "fanout_gate", &mut warnings),
            autonomy,
            await_timeout_ms: clamp_keyed(
                self.await_timeout_ms, AWAIT_TIMEOUT_MS, "await_timeout_ms", &mut warnings,
            ),
            token_budget_day: clamp_keyed(self.token_budget_day, 0, "token_budget_day", &mut warnings),
            heartbeat_sample_ms: clamp_keyed(
                self.heartbeat_sample_ms, HEARTBEAT_SAMPLE_MS, "heartbeat_sample_ms", &mut warnings,
            ),
            stall_warn_samples: clamp_keyed(
                self.stall_warn_samples, STALL_WARN_SAMPLES, "stall_warn_samples", &mut warnings,
            ),
            stall_breaker_samples: clamp_keyed(
                self.stall_breaker_samples, STALL_BREAKER_SAMPLES, "stall_breaker_samples",
                &mut warnings,
            ),
            retention_timeout_ms: clamp_keyed(
                self.retention_timeout_ms, RETENTION_TIMEOUT_MS, "retention_timeout_ms", &mut warnings,
            ),
            delivery_max_attempts: clamp_keyed(
                self.delivery_max_attempts, DELIVERY_MAX_ATTEMPTS, "delivery_max_attempts",
                &mut warnings,
            ),
            delivery_backoff_base_ms: clamp_keyed(
                self.delivery_backoff_base_ms, DELIVERY_BACKOFF_BASE_MS, "delivery_backoff_base_ms",
                &mut warnings,
            ),
            breaker_threshold: clamp_keyed(
                self.breaker_threshold, BREAKER_THRESHOLD, "breaker_threshold", &mut warnings,
            ),
        };
        let max_cycle_revisits =
            clamp_keyed(self.max_cycle_revisits, MAX_CYCLE_REVISITS, "max_cycle_revisits", &mut warnings);
        let scrollback_retention_days = clamp_keyed(
            self.scrollback_retention_days, DEFAULT_RETENTION_DAYS, "scrollback_retention_days",
            &mut warnings,
        );
        ResolvedParams {
            router_config,
            effort: self.effort.unwrap_or_default(),
            model: self.model,
            scrollback_retention_days,
            max_cycle_revisits,
            warnings,
        }
    }

    /// Aplica a ESTA camada um par (chave, valor-serializado) vindo de um `SystemParamsChanged`.
    /// Valor não-parseável para o tipo do campo vira `None` (resiliência de replay — o `lina params
    /// set` valida a faixa na ESCRITA, F3-0-5; um log corrompido não pode derrubar a reconstrução);
    /// chave desconhecida é ignorada (ADITIVO: um evento de versão futura não quebra o replay).
    fn set_from_event(&mut self, key: &str, value: &str) {
        match key {
            "delegation_budget" => self.delegation_budget = value.parse().ok(),
            "max_depth" => self.max_depth = value.parse().ok(),
            "fanout_gate" => self.fanout_gate = value.parse().ok(),
            "max_spawns_per_turn" => self.max_spawns_per_turn = value.parse().ok(),
            "max_cycle_revisits" => self.max_cycle_revisits = value.parse().ok(),
            "dedupe_window_ms" => self.dedupe_window_ms = value.parse().ok(),
            "await_timeout_ms" => self.await_timeout_ms = value.parse().ok(),
            "retention_timeout_ms" => self.retention_timeout_ms = value.parse().ok(),
            "delivery_max_attempts" => self.delivery_max_attempts = value.parse().ok(),
            "delivery_backoff_base_ms" => self.delivery_backoff_base_ms = value.parse().ok(),
            "breaker_threshold" => self.breaker_threshold = value.parse().ok(),
            "heartbeat_sample_ms" => self.heartbeat_sample_ms = value.parse().ok(),
            "stall_warn_samples" => self.stall_warn_samples = value.parse().ok(),
            "stall_breaker_samples" => self.stall_breaker_samples = value.parse().ok(),
            "token_budget_day" => self.token_budget_day = value.parse().ok(),
            "scrollback_retention_days" => self.scrollback_retention_days = value.parse().ok(),
            "model" => self.model = Some(value.to_string()),
            // Reusa o serde lowercase de `Effort` como fonte única do vocabulário low/medium/high.
            "effort" => {
                self.effort =
                    serde_json::from_value(serde_json::Value::String(value.to_string())).ok();
            }
            _ => {}
        }
    }
}

/// As 4 camadas de [`SystemParams`] reconstruídas do event log (projeção event-sourced — invariante
/// #4). Espelha o `CostLedger` (`router.rs`): `replay` varre os `SystemParamsChanged` em ordem de
/// `seq` e, para cada um, aplica `(key, new)` à camada `scope`. O último evento de uma mesma
/// `(scope, key)` vence (ordem do log). Nada de campo efêmero: o `RouterConfig` é derivável só do log.
#[derive(Debug, Clone, Default)]
pub struct ParamsLedger {
    pub global: SystemParams,
    pub workspace: SystemParams,
    pub preset: SystemParams,
    pub terminal: SystemParams,
}

impl ParamsLedger {
    /// Reconstrói as camadas varrendo o log inteiro.
    ///
    /// # Errors
    /// Falha ao ler o event store.
    pub fn replay(store: &EventStore) -> Result<Self, StoreError> {
        Ok(Self::from_records(&store.events()?))
    }

    /// Reconstrói as camadas a partir de uma fatia de [`EventRecord`] (forma pura, sem I/O). É o
    /// caminho do verbo SÓ-LEITURA `lina params show` (F3-0-5): ele monta o ledger dos records do
    /// **JSONL** (espelho de DR), NUNCA do SQLite — doutrina do verbo read-only (`lina.rs:1139`);
    /// [`resolve_from_store`] é quem lê o SQLite e por isso não serve ao `show`.
    #[must_use]
    pub fn from_records(records: &[EventRecord]) -> Self {
        let mut ledger = Self::default();
        for rec in records {
            if rec.kind != "SystemParamsChanged" {
                continue;
            }
            let (Some(scope), Some(key), Some(new)) = (
                rec.payload.get("scope").and_then(serde_json::Value::as_str),
                rec.payload.get("key").and_then(serde_json::Value::as_str),
                rec.payload.get("new").and_then(serde_json::Value::as_str),
            ) else {
                continue; // evento malformado não derruba o replay (best-effort)
            };
            let layer = match scope {
                "global" => &mut ledger.global,
                "workspace" => &mut ledger.workspace,
                "preset" => &mut ledger.preset,
                "terminal" => &mut ledger.terminal,
                _ => continue, // escopo desconhecido (versão futura) → ignora (aditivo)
            };
            layer.set_from_event(key, new);
        }
        ledger
    }

    /// Funde as 4 camadas na precedência canônica (`global ⊕ workspace ⊕ preset ⊕ terminal`) e
    /// produz o [`ResolvedParams`] denso (default + clamp). `autonomy` vem do bootstrap.
    #[must_use]
    pub fn resolve(self, autonomy: AutonomyLevel) -> ResolvedParams {
        self.global
            .overlay(self.workspace)
            .overlay(self.preset)
            .overlay(self.terminal)
            .into_resolved(autonomy)
    }
}

/// Reconstrói o [`ResolvedParams`] (logo, o [`RouterConfig`]) **só por replay do event log** — sem
/// reler `params.json` (invariante #4; critério de aceite (b) da F3-0-2). É o que o seam do
/// `bridge.rs` passa a chamar no lugar do `..RouterConfig::default()`. Espelha `CostLedger::replay`.
///
/// # Errors
/// Falha ao ler o event store.
pub fn resolve_from_store(
    store: &EventStore,
    autonomy: AutonomyLevel,
) -> Result<ResolvedParams, StoreError> {
    Ok(ParamsLedger::replay(store)?.resolve(autonomy))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::DomainEvent;
    use uuid::Uuid;

    /// Lê todos os 15 campos do `RouterConfig` resolvido e confere contra um esperado, campo a
    /// campo (o `RouterConfig` não deriva `PartialEq` — não posso usar `==`).
    fn assert_router_config_eq(got: &RouterConfig, want: &RouterConfig) {
        assert_eq!(got.dedupe_window_ms, want.dedupe_window_ms, "dedupe_window_ms");
        assert_eq!(got.max_depth, want.max_depth, "max_depth");
        assert_eq!(got.delegation_budget, want.delegation_budget, "delegation_budget");
        assert_eq!(got.max_spawns_per_turn, want.max_spawns_per_turn, "max_spawns_per_turn");
        assert_eq!(got.fanout_gate, want.fanout_gate, "fanout_gate");
        assert_eq!(got.autonomy, want.autonomy, "autonomy");
        assert_eq!(got.await_timeout_ms, want.await_timeout_ms, "await_timeout_ms");
        assert_eq!(got.token_budget_day, want.token_budget_day, "token_budget_day");
        assert_eq!(got.heartbeat_sample_ms, want.heartbeat_sample_ms, "heartbeat_sample_ms");
        assert_eq!(got.stall_warn_samples, want.stall_warn_samples, "stall_warn_samples");
        assert_eq!(got.stall_breaker_samples, want.stall_breaker_samples, "stall_breaker_samples");
        assert_eq!(got.retention_timeout_ms, want.retention_timeout_ms, "retention_timeout_ms");
        assert_eq!(got.delivery_max_attempts, want.delivery_max_attempts, "delivery_max_attempts");
        assert_eq!(
            got.delivery_backoff_base_ms, want.delivery_backoff_base_ms,
            "delivery_backoff_base_ms"
        );
        assert_eq!(got.breaker_threshold, want.breaker_threshold, "breaker_threshold");
    }

    /// CRITÉRIO (compat total): sem nenhuma camada (4× `default()`), `resolve` cai **exatamente**
    /// nos defaults atuais do `RouterConfig` — nenhum workspace existente muda de comportamento.
    #[test]
    fn sem_camada_cai_nos_defaults_atuais_compat_total() {
        let r = SystemParams::resolve(
            SystemParams::default(),
            SystemParams::default(),
            SystemParams::default(),
            SystemParams::default(),
            AutonomyLevel::Assisted,
        );
        assert_router_config_eq(&r.router_config, &RouterConfig::default());
        assert_eq!(r.effort, Effort::Medium, "effort default = Medium");
        assert_eq!(r.model, None, "model default = None");
        assert_eq!(r.max_cycle_revisits, MAX_CYCLE_REVISITS);
        assert_eq!(r.scrollback_retention_days, DEFAULT_RETENTION_DAYS);
        assert!(r.warnings.is_empty(), "sem override válido = sem WARN");
    }

    /// CRITÉRIO (precedência): quatro camadas, só a de **terminal** opina `fanout_gate=8` →
    /// `resolve` usa o de terminal; os demais campos caem no default.
    #[test]
    fn quatro_camadas_so_terminal_opina_fanout_gate_resolve_usa_terminal() {
        let terminal = SystemParams { fanout_gate: Some(8), ..Default::default() };
        let r = SystemParams::resolve(
            SystemParams::default(),
            SystemParams::default(),
            SystemParams::default(),
            terminal,
            AutonomyLevel::Assisted,
        );
        assert_eq!(r.router_config.fanout_gate, 8, "terminal opinou fanout_gate");
        assert_eq!(r.router_config.delegation_budget, DELEGATION_BUDGET, "resto = default");
        assert!(r.warnings.is_empty());
    }

    /// `overlay`: campo `Some` de `over` vence; `None` herda da base.
    #[test]
    fn overlay_over_ganha_none_herda() {
        let base = SystemParams { delegation_budget: Some(10), fanout_gate: Some(5), ..Default::default() };
        let over = SystemParams { fanout_gate: Some(12), ..Default::default() };
        let merged = base.overlay(over);
        assert_eq!(merged.fanout_gate, Some(12), "over ganha");
        assert_eq!(merged.delegation_budget, Some(10), "None de over herda da base");
    }

    /// Precedência ao longo da cadeia: uma camada baixa (global) opina e nenhuma acima sobrescreve
    /// → o valor de global atravessa até o `RouterConfig`.
    #[test]
    fn camada_baixa_atravessa_quando_ninguem_sobrescreve() {
        let global = SystemParams { delegation_budget: Some(16), ..Default::default() };
        let r = SystemParams::resolve(
            global,
            SystemParams::default(),
            SystemParams::default(),
            SystemParams::default(),
            AutonomyLevel::Assisted,
        );
        assert_eq!(r.router_config.delegation_budget, 16);
    }

    /// CRITÉRIO 3 (faixa sem panic): `delegation_budget=9999` (acima de 64) → clamp para 64 + 1
    /// WARN nomeado; zero panic.
    #[test]
    fn clamp_fora_da_faixa_gera_warn_sem_panic() {
        let ws = SystemParams { delegation_budget: Some(9999), ..Default::default() };
        let r = SystemParams::resolve(
            SystemParams::default(),
            ws,
            SystemParams::default(),
            SystemParams::default(),
            AutonomyLevel::Assisted,
        );
        assert_eq!(r.router_config.delegation_budget, 64, "clampado ao teto");
        assert_eq!(r.warnings.len(), 1, "exatamente 1 WARN");
        assert_eq!(r.warnings[0].key, "delegation_budget");
        assert_eq!(r.warnings[0].requested, "9999");
        assert_eq!(r.warnings[0].clamped, "64");
    }

    /// Clamp também pelo piso da faixa (ex.: `max_depth=0` < min 1 → 1).
    #[test]
    fn clamp_pelo_piso_da_faixa() {
        let ws = SystemParams { max_depth: Some(0), ..Default::default() };
        let r = SystemParams::resolve(
            SystemParams::default(),
            ws,
            SystemParams::default(),
            SystemParams::default(),
            AutonomyLevel::Assisted,
        );
        assert_eq!(r.router_config.max_depth, 1);
        assert_eq!(r.warnings.len(), 1);
        assert_eq!(r.warnings[0].key, "max_depth");
    }

    /// `effort`: `None` → Medium; uma camada superior sobrescreve.
    #[test]
    fn effort_default_medium_e_terminal_sobrescreve() {
        let terminal = SystemParams { effort: Some(Effort::High), ..Default::default() };
        let r = SystemParams::resolve(
            SystemParams::default(),
            SystemParams::default(),
            SystemParams::default(),
            terminal,
            AutonomyLevel::Assisted,
        );
        assert_eq!(r.effort, Effort::High);
    }

    /// `autonomy` vem do parâmetro (bootstrap), não de `params.json` — `Manual` chega ao config.
    #[test]
    fn autonomy_vem_do_bootstrap_nao_do_params() {
        let r = SystemParams::resolve(
            SystemParams::default(),
            SystemParams::default(),
            SystemParams::default(),
            SystemParams::default(),
            AutonomyLevel::Manual,
        );
        assert_eq!(r.router_config.autonomy, AutonomyLevel::Manual);
    }

    /// `Effort` é contrato neutro `lowercase` (serde) e tem default `Medium`.
    #[test]
    fn effort_serde_lowercase_e_default_medium() {
        assert_eq!(Effort::default(), Effort::Medium);
        assert_eq!(serde_json::to_string(&Effort::High).unwrap(), "\"high\"");
        assert_eq!(serde_json::from_str::<Effort>("\"low\"").unwrap(), Effort::Low);
    }

    /// ADITIVO: um `params.json` que só conhece um campo desserializa o resto como `None`
    /// (`#[serde(default)]`) — carga de config antiga nunca quebra.
    #[test]
    fn deserializa_parcial_com_serde_default() {
        let p: SystemParams = serde_json::from_str(r#"{"fanout_gate": 7}"#).unwrap();
        assert_eq!(p.fanout_gate, Some(7));
        assert_eq!(p.delegation_budget, None);
        assert_eq!(p.effort, None);
    }

    // ─────────────── F3-0-2: RouterConfig como PROJEÇÃO do event log ───────────────

    /// Event store temporário (convenção do crate — espelha o helper de `events.rs` tests).
    struct TempDir(std::path::PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!("lina-params-{tag}-{}", Uuid::now_v7()));
            std::fs::create_dir_all(&p).expect("criar tempdir");
            Self(p)
        }
        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Monta um `EventRecord` de `SystemParamsChanged` para os testes puros de `from_records`.
    fn spc(scope: &str, key: &str, new: &str) -> EventRecord {
        EventRecord {
            seq: 0,
            ts: 0,
            kind: "SystemParamsChanged".to_string(),
            version: 1,
            payload: serde_json::json!({ "scope": scope, "key": key, "new": new }),
        }
    }

    /// Precedência no replay: terminal (camada alta) vence workspace (camada baixa).
    #[test]
    fn replay_precedencia_terminal_vence_workspace() {
        let recs = vec![spc("workspace", "fanout_gate", "8"), spc("terminal", "fanout_gate", "12")];
        let r = ParamsLedger::from_records(&recs).resolve(AutonomyLevel::Assisted);
        assert_eq!(r.router_config.fanout_gate, 12, "terminal > workspace");
    }

    /// Na MESMA (scope,key), o último evento do log vence (ordem de seq).
    #[test]
    fn replay_last_write_wins_na_mesma_camada() {
        let recs = vec![spc("workspace", "fanout_gate", "8"), spc("workspace", "fanout_gate", "5")];
        let r = ParamsLedger::from_records(&recs).resolve(AutonomyLevel::Assisted);
        assert_eq!(r.router_config.fanout_gate, 5, "último da mesma camada/chave vence");
    }

    /// ADITIVO: um `scope` desconhecido (evento de versão futura) é ignorado — não aplica nem quebra.
    #[test]
    fn replay_escopo_desconhecido_e_ignorado() {
        let recs = vec![spc("galaxy", "fanout_gate", "9")];
        let r = ParamsLedger::from_records(&recs).resolve(AutonomyLevel::Assisted);
        assert_eq!(r.router_config.fanout_gate, FANOUT_GATE, "scope desconhecido não aplica");
    }

    /// ADITIVO: uma `key` desconhecida é ignorada — replay de evento de versão futura é seguro.
    #[test]
    fn replay_chave_desconhecida_e_ignorada() {
        let recs = vec![spc("workspace", "quantum_flux", "42")];
        let r = ParamsLedger::from_records(&recs).resolve(AutonomyLevel::Assisted);
        assert_eq!(r.router_config.fanout_gate, FANOUT_GATE);
        assert!(r.warnings.is_empty());
    }

    /// O replay passa pelo MESMO clamp do `resolve`: um valor fora da faixa no log é saneado.
    #[test]
    fn replay_clampa_valor_fora_da_faixa() {
        let recs = vec![spc("workspace", "delegation_budget", "9999")];
        let r = ParamsLedger::from_records(&recs).resolve(AutonomyLevel::Assisted);
        assert_eq!(r.router_config.delegation_budget, 64, "replay clampa ao teto");
        assert_eq!(r.warnings.len(), 1);
        assert_eq!(r.warnings[0].key, "delegation_budget");
    }

    /// CRITÉRIO (b): emitir `SystemParamsChanged{fanout_gate=8}` e reconstruir o `RouterConfig`
    /// SÓ por replay do log (sem reler `params.json`) → `fanout_gate == 8`.
    #[test]
    fn resolve_from_store_reconstroi_fanout_gate_8_so_do_log() {
        let tmp = TempDir::new("replay-fanout");
        let mut store = EventStore::open(tmp.path()).expect("abrir store");
        store
            .append(&DomainEvent::SystemParamsChanged {
                key: "fanout_gate".to_string(),
                scope: "workspace".to_string(),
                new: "8".to_string(),
                target: None,
                old: "3".to_string(),
                by: Some("human".to_string()),
            })
            .expect("append");
        let r = resolve_from_store(&store, AutonomyLevel::Assisted).expect("resolve");
        assert_eq!(r.router_config.fanout_gate, 8, "reconstruído só do log");
    }

    /// CRITÉRIO (compat): sem nenhum evento nem params.json, o replay cai EXATAMENTE nos defaults.
    #[test]
    fn resolve_from_store_vazio_cai_nos_defaults() {
        let tmp = TempDir::new("replay-vazio");
        let store = EventStore::open(tmp.path()).expect("abrir store");
        let r = resolve_from_store(&store, AutonomyLevel::Assisted).expect("resolve");
        assert_router_config_eq(&r.router_config, &RouterConfig::default());
        assert!(r.warnings.is_empty());
    }

    // ─────────────── F3-0-5-core: validate_range (fonte única das faixas p/ o bin) ───────────────

    /// `lina params set` RECUSA valor fora da faixa (≠ load, que clampa) — com a faixa na mensagem.
    #[test]
    fn validate_range_aceita_dentro_recusa_fora() {
        assert!(validate_range("delegation_budget", "8").is_ok());
        assert!(validate_range("delegation_budget", "64").is_ok(), "teto inclusivo");
        assert!(validate_range("delegation_budget", "1").is_ok(), "piso inclusivo");
        let e = validate_range("delegation_budget", "9999").unwrap_err();
        assert!(e.contains("9999") && e.contains("64"), "erro nomeia valor e faixa: {e}");
        assert!(validate_range("delegation_budget", "0").is_err(), "abaixo do piso");
    }

    /// `effort`/`model` têm vocabulário próprio; chave/valor inválidos são recusados.
    #[test]
    fn validate_range_effort_model_e_desconhecidos() {
        assert!(validate_range("effort", "high").is_ok());
        assert!(validate_range("effort", "altissimo").is_err(), "effort fora do vocabulário");
        assert!(validate_range("model", "opus").is_ok(), "model aceita alias (CLI Profile valida)");
        assert!(validate_range("inexistente", "1").is_err(), "chave desconhecida recusada");
        assert!(validate_range("fanout_gate", "abc").is_err(), "valor não-numérico recusado");
    }

    /// F3-0-7 §C: a classificação gateia SÓ o ENFRAQUECIMENTO (reduzir abaixo do default) dos laços
    /// balanceadores — aumentar, default-exato e parâmetros não-defensivos são `Routine`.
    #[test]
    fn param_change_action_class_gates_only_weakening_of_balancing_loops() {
        // reduzir laço balanceador abaixo do default → GatedHard (os 3 da F3-0-7 + os 2 do R4).
        assert_eq!(param_change_action_class("delegation_budget", 4), ActionClass::GatedHard);
        assert_eq!(param_change_action_class("breaker_threshold", 2), ActionClass::GatedHard);
        assert_eq!(param_change_action_class("stall_breaker_samples", 4), ActionClass::GatedHard);
        assert_eq!(param_change_action_class("fanout_gate", 1), ActionClass::GatedHard);
        assert_eq!(param_change_action_class("max_depth", 2), ActionClass::GatedHard);
        // AUMENTAR/fortalecer NÃO é GatedHard (subir o teto do laço não desliga o breaker).
        assert_eq!(param_change_action_class("delegation_budget", 16), ActionClass::Routine);
        // No default EXATO não há redução.
        assert_eq!(param_change_action_class("breaker_threshold", 5), ActionClass::Routine);
        // Parâmetro NÃO-defensivo nunca escala por esta regra (tem gates próprios — ADR 0005).
        assert_eq!(param_change_action_class("scrollback_retention_days", 1), ActionClass::Routine);
        assert_eq!(param_change_action_class("token_budget_day", 0), ActionClass::Routine);
    }

    /// A faixa de `validate_range` é a MESMA do clamp do `resolve` (fonte única `param_range`): o
    /// que o `set` recusa, o `resolve` clampa para a mesma borda — sem 16 faixas duplicadas.
    #[test]
    fn validate_range_e_clamp_compartilham_a_faixa() {
        // 9999 é recusado pelo set...
        assert!(validate_range("delegation_budget", "9999").is_err());
        // ...e clampado para 64 (o teto) pelo resolve — mesma borda.
        let ws = SystemParams { delegation_budget: Some(9999), ..Default::default() };
        let r = SystemParams::resolve(
            SystemParams::default(),
            ws,
            SystemParams::default(),
            SystemParams::default(),
            AutonomyLevel::Assisted,
        );
        assert_eq!(r.router_config.delegation_budget, 64);
    }
}
