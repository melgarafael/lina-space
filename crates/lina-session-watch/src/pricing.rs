//! **Tabela de preço por modelo (USD/Mtok) → ESTIMATIVA de custo a partir de `message.usage`.**
//!
//! Por que existe: o formato ATUAL de session-file do Claude Code **não grava `costUSD`**
//! (verificado em arquivos reais de 2026-06 — nenhuma linha o traz); só `message.usage`
//! (tokens). Sem derivar custo de tokens×preço, `Session::cost_usd` fica 0 para sempre e o
//! dashboard (F1-1-5) mostra "sem estimativa de custo ainda" eternamente — o bug do .app.
//!
//! Honestidade (13.5): tudo aqui é **ESTIMATIVA** (a fonte JSONL já marca
//! `cost_estimated = true`; o card exibe `~`/`(estimado)`). Modelo fora da tabela →
//! [`estimate_usage_cost`] devolve `None` → o consumidor mantém o fallback honesto
//! ("sem estimativa"), **nunca chuta preço**.
//!
//! Fonte dos números: tabela oficial de pricing da API Anthropic (cache 2026-05-26):
//! Opus 4.5–4.8 = $5/$25 · Sonnet 4.x = $3/$15 · Haiku 4.5 = $1/$5 (in/out por Mtok);
//! cache read = 0,1× input · cache write 5m = 1,25× · cache write 1h = 2× input.
//! Atualizar preço = editar SÓ esta tabela (função pura; nada de rede/estado).

/// `(substring do id do modelo, USD/Mtok input, USD/Mtok output)` — a PRIMEIRA que casar
/// vence (por isso as famílias mais específicas vêm antes). Substring cobre sufixos reais
/// de id (`-20251101`, `[1m]`) sem dicionário por release.
const RATES: &[(&str, f64, f64)] = &[
    // Opus 4.5+ ($5/$25). Antes de "opus-4-" legado p/ não casar errado.
    ("opus-4-5", 5.0, 25.0),
    ("opus-4-6", 5.0, 25.0),
    ("opus-4-7", 5.0, 25.0),
    ("opus-4-8", 5.0, 25.0),
    // Opus 4.0/4.1 legado ($15/$75) — ids `claude-opus-4-1…`/`claude-opus-4-2025…`.
    ("opus-4-0", 15.0, 75.0),
    ("opus-4-1", 15.0, 75.0),
    ("opus-4-2025", 15.0, 75.0),
    // Sonnet 4.x e 3.5/3.7 compartilham $3/$15.
    ("sonnet-4", 3.0, 15.0),
    ("sonnet-3-5", 3.0, 15.0),
    ("sonnet-3-7", 3.0, 15.0),
    ("haiku-4-5", 1.0, 5.0),
];

/// Multiplicadores de cache sobre o preço de INPUT (contrato da API Anthropic).
const CACHE_READ_MULT: f64 = 0.1;
const CACHE_WRITE_5M_MULT: f64 = 1.25;
const CACHE_WRITE_1H_MULT: f64 = 2.0;

/// Preço `(USD/Mtok input, USD/Mtok output)` do modelo, se a família é conhecida.
#[must_use]
pub fn rates_for_model(model: &str) -> Option<(f64, f64)> {
    RATES
        .iter()
        .find(|(needle, _, _)| model.contains(needle))
        .map(|&(_, input, output)| (input, output))
}

/// **Custo estimado (USD) de UM bloco `message.usage`** do session-file — função pura.
///
/// Lê `input_tokens`/`output_tokens`/`cache_read_input_tokens` e o breakdown de cache
/// write (`cache_creation.ephemeral_{5m,1h}_input_tokens`; sem breakdown, o total
/// `cache_creation_input_tokens` é cobrado a 1,25× — o TTL mais barato: estimativa
/// conservadora p/ baixo, nunca infla). `None` = modelo fora da tabela (sem chute).
#[must_use]
pub fn estimate_usage_cost(model: &str, usage: &serde_json::Value) -> Option<f64> {
    let (in_mtok, out_mtok) = rates_for_model(model)?;
    let per_in = in_mtok / 1e6;
    let per_out = out_mtok / 1e6;
    let n = |v: &serde_json::Value, k: &str| -> f64 {
        v.get(k).and_then(serde_json::Value::as_u64).unwrap_or(0) as f64
    };

    let mut cost = n(usage, "input_tokens") * per_in
        + n(usage, "output_tokens") * per_out
        + n(usage, "cache_read_input_tokens") * per_in * CACHE_READ_MULT;

    let write_total = n(usage, "cache_creation_input_tokens");
    let breakdown = usage.get("cache_creation");
    let (w5m, w1h) = breakdown
        .map(|cc| {
            (
                n(cc, "ephemeral_5m_input_tokens"),
                n(cc, "ephemeral_1h_input_tokens"),
            )
        })
        .unwrap_or((0.0, 0.0));
    if w5m + w1h > 0.0 {
        cost += w5m * per_in * CACHE_WRITE_5M_MULT + w1h * per_in * CACHE_WRITE_1H_MULT;
    } else {
        cost += write_total * per_in * CACHE_WRITE_5M_MULT;
    }
    Some(cost)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Famílias conhecidas resolvem (inclusive com sufixos reais de id); desconhecida → None.
    #[test]
    fn rates_match_model_families_and_reject_unknown() {
        assert_eq!(rates_for_model("claude-opus-4-8"), Some((5.0, 25.0)));
        assert_eq!(rates_for_model("claude-opus-4-8[1m]"), Some((5.0, 25.0)));
        assert_eq!(
            rates_for_model("claude-opus-4-5-20251101"),
            Some((5.0, 25.0))
        );
        assert_eq!(
            rates_for_model("claude-opus-4-1-20250805"),
            Some((15.0, 75.0))
        );
        assert_eq!(rates_for_model("claude-sonnet-4-6"), Some((3.0, 15.0)));
        assert_eq!(
            rates_for_model("claude-haiku-4-5-20251001"),
            Some((1.0, 5.0))
        );
        assert_eq!(rates_for_model("futuro-llm-99"), None, "nunca chuta preço");
    }

    /// Conta fechada: in/out + cache read (0,1×) + write com breakdown 5m (1,25×) e 1h (2×).
    #[test]
    fn estimate_covers_cache_read_and_write_ttls() {
        let usage: serde_json::Value = serde_json::json!({
            "input_tokens": 1_000_000,
            "output_tokens": 100_000,
            "cache_read_input_tokens": 2_000_000,
            "cache_creation_input_tokens": 300_000,
            "cache_creation": {
                "ephemeral_5m_input_tokens": 100_000,
                "ephemeral_1h_input_tokens": 200_000
            }
        });
        // opus-4-8: 1Mtok in = $5; 0,1Mtok out = $2,50; 2Mtok read = 2×$0,50 = $1;
        // write 0,1Mtok×1,25×$5 = $0,625; 0,2Mtok×2×$5 = $2. Total $11,125.
        let cost = estimate_usage_cost("claude-opus-4-8", &usage).expect("modelo conhecido");
        assert!((cost - 11.125).abs() < 1e-9, "custo {cost}");
    }

    /// Sem breakdown de TTL, o write total cobra 1,25× (estimativa p/ baixo, nunca infla).
    #[test]
    fn estimate_without_ttl_breakdown_uses_cheapest_write() {
        let usage = serde_json::json!({
            "input_tokens": 0, "output_tokens": 0,
            "cache_creation_input_tokens": 1_000_000
        });
        let cost = estimate_usage_cost("claude-opus-4-8", &usage).expect("conhecido");
        assert!(
            (cost - 6.25).abs() < 1e-9,
            "1Mtok×1,25×$5 = $6,25; got {cost}"
        );
    }

    /// Modelo desconhecido → None (o consumidor exibe "sem estimativa", não inventa).
    #[test]
    fn estimate_unknown_model_is_none() {
        let usage = serde_json::json!({"input_tokens": 10});
        assert_eq!(estimate_usage_cost("misterioso-1", &usage), None);
    }
}
