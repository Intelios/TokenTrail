use std::collections::HashMap;
use std::sync::OnceLock;

use crate::models::{Source, UsageEvent};

// Approximate API list prices (USD per million tokens) from a LiteLLM-style
// snapshot. Every cost figure in the app is an "API-equivalent estimate",
// not a measure of actual subscription spend.
const PRICING_JSON: &str = include_str!("../pricing/pricing.json");

#[derive(Debug, Clone, Copy, serde::Deserialize)]
struct Rates {
    input: f64,
    output: f64,
    cache_read: f64,
    cache_write: f64,
}

#[derive(Debug, serde::Deserialize)]
struct Table {
    models: HashMap<String, Rates>,
    families: Vec<Family>,
}

#[derive(Debug, serde::Deserialize)]
struct Family {
    prefix: String,
    rates: Rates,
}

fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| serde_json::from_str(PRICING_JSON).expect("embedded pricing.json is valid"))
}

/// Strip harness decorations and provider prefixes: "claude-opus-5[ffe]" and
/// "anthropic/claude-sonnet-4.5" both reduce to their family-matchable base
/// (the last path segment, minus any "[suffix]" decoration).
pub fn normalize_model(model: &str) -> String {
    let base = model.rsplit('/').next().unwrap_or(model);
    let base = base.split('[').next().unwrap_or(base);
    base.trim().to_ascii_lowercase()
}

fn rates_for(model: &str) -> Option<Rates> {
    let t = table();
    let full = model
        .split('[')
        .next()
        .unwrap_or(model)
        .trim()
        .to_ascii_lowercase();
    if full.is_empty() {
        return None;
    }
    // The full string ("model/variant" order) and the provider-stripped base
    // ("provider/model" order) are the two shapes real harnesses produce.
    let base = normalize_model(&full);
    let mut candidates: Vec<&str> = vec![full.as_str()];
    if base != full {
        candidates.push(base.as_str());
    }
    for c in &candidates {
        if let Some(r) = t.models.get(*c) {
            return Some(*r);
        }
    }
    for c in &candidates {
        if let Some(r) = t.families.iter().find(|f| c.starts_with(&f.prefix)) {
            return Some(r.rates);
        }
    }
    None
}

/// API-equivalent cost in USD of one model request, from the bundled list
/// prices. Returns `None` when the model is unknown or missing.
pub fn cost_for(
    source: Source,
    model: Option<&str>,
    input_tokens: i64,
    output_tokens: i64,
    reasoning_tokens: Option<i64>,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
) -> Option<f64> {
    let r = rates_for(model?)?;
    // Providers bill thinking tokens at the output rate. Most collectors report
    // output including thinking (already covered by r.output); Antigravity is
    // the one that subtracts thinking from output_tokens, so its reasoning
    // column is billed here to keep costs comparable across harnesses.
    let reasoning = match source {
        Source::Antigravity => reasoning_tokens.unwrap_or(0),
        _ => 0,
    };
    Some(
        (input_tokens as f64 * r.input
            + cache_write_tokens as f64 * r.cache_write
            + cache_read_tokens as f64 * r.cache_read
            + (output_tokens + reasoning) as f64 * r.output)
            / 1_000_000.0,
    )
}

pub fn cost_usd(model: Option<&str>, e: &UsageEvent) -> Option<f64> {
    cost_for(
        e.source,
        model,
        e.input_tokens,
        e.output_tokens,
        e.reasoning_tokens,
        e.cache_read_tokens,
        e.cache_write_tokens,
    )
}

/// FNV-1a hash of the embedded pricing table. Startup compares this against a
/// stored watermark to detect pricing updates and reprice the stored history.
pub fn pricing_fingerprint() -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in PRICING_JSON.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(model: Option<&str>, input: i64, cw: i64, cr: i64, out: i64) -> UsageEvent {
        UsageEvent {
            source: Source::Zcode,
            source_event_id: "x".into(),
            ts: 0,
            session_id: None,
            project: None,
            provider: None,
            model: model.map(String::from),
            input_tokens: input,
            output_tokens: out,
            reasoning_tokens: None,
            cache_read_tokens: cr,
            cache_write_tokens: cw,
            duration_ms: None,
            ttft_ms: None,
            is_subagent: false,
        }
    }

    #[test]
    fn matches_families() {
        assert!(rates_for("claude-opus-5").is_some());
        assert!(rates_for("Claude-Sonnet-4.5[ffe]").is_some());
        assert!(rates_for("deepseek-v4-flash:0731-cloud").is_some());
        assert!(rates_for("gpt-5.6-luna").is_some());
        assert!(rates_for("totally-unknown-model").is_none());
        assert!(rates_for("").is_none());
    }

    #[test]
    fn resolves_provider_prefixes() {
        // "provider/model" from OpenCode-style harnesses must price the model.
        assert_eq!(rates_for("anthropic/claude-sonnet-4.5").map(|r| r.output), Some(15.0));
        assert_eq!(rates_for("qwen-cloud/qwen3.8-max").map(|r| r.input), Some(2.0));
        // "model/variant" style names still match through the full string.
        assert_eq!(rates_for("gpt-5.6-sol/long-context").map(|r| r.input), Some(5.0));
    }

    #[test]
    fn specific_prefixes_beat_catch_alls() {
        // gpt-5.6-sol is far pricier than the generic gpt-5 / gpt families.
        assert_eq!(rates_for("gpt-5.6-sol").map(|r| r.input), Some(5.0));
        assert_eq!(rates_for("gpt-5.6-luna").map(|r| r.input), Some(0.2));
        assert_eq!(rates_for("gpt-5.5").map(|r| r.input), Some(5.0));
        assert_eq!(rates_for("gpt-5.5").map(|r| r.output), Some(30.0));
        // claude-opus-4 keeps the older Opus 4.x rate; opus-5 uses the new one.
        assert_eq!(rates_for("claude-opus-4-8").map(|r| r.input), Some(15.0));
        assert_eq!(rates_for("claude-opus-5").map(|r| r.input), Some(5.0));
        // Sonnet 5 dropped to $2/$10; older Sonnet 4.x keeps the $3/$15 rate.
        assert_eq!(rates_for("claude-sonnet-5").map(|r| r.input), Some(2.0));
        assert_eq!(rates_for("claude-sonnet-5").map(|r| r.output), Some(10.0));
        assert_eq!(rates_for("claude-sonnet-4.5").map(|r| r.input), Some(3.0));
        // MiMo-V2.5-Pro has its own rates above the generic mimo catch-all.
        assert_eq!(rates_for("mimo-v2.5-pro").map(|r| r.input), Some(0.435));
        assert_eq!(rates_for("mimo-v2.5-pro").map(|r| r.output), Some(0.87));
        assert_eq!(rates_for("mimo-v2.5-pro").map(|r| r.cache_read), Some(0.0036));
        // deepseek-v4-pro outranks the deepseek catch-all; the free variant is $0.
        assert_eq!(rates_for("deepseek-v4-pro:0813-cloud").map(|r| r.input), Some(1.32));
        assert_eq!(rates_for("deepseek-v4-flash-free").map(|r| r.input), Some(0.0));
    }

    #[test]
    fn computes_cost() {
        // 1M input + 1M output on a $3/$15 model = $18
        let e = event(Some("claude-sonnet-4.5"), 1_000_000, 0, 0, 1_000_000);
        let cost = cost_usd(e.model.as_deref(), &e).unwrap();
        assert!((cost - 18.0).abs() < 1e-9);
        assert!(cost_usd(None, &e).is_none());
    }

    #[test]
    fn bills_antigravity_reasoning_as_output() {
        let mut e = event(Some("gemini-3.7-flash"), 0, 0, 0, 200);
        e.source = Source::Antigravity;
        e.reasoning_tokens = Some(100);
        // 300 total output tokens billed at $3.75/M = $0.001125
        let cost = cost_usd(e.model.as_deref(), &e).unwrap();
        assert!((cost - 0.001125).abs() < 1e-12);
        // The same event under a source that reports output including thinking
        // (e.g. Gemini CLI) is not double-billed for reasoning.
        e.source = Source::Gemini;
        let cost = cost_usd(e.model.as_deref(), &e).unwrap();
        assert!((cost - 0.00075).abs() < 1e-12);
    }

    #[test]
    fn muse_spark_prices() {
        // Muse Spark 1.2 (and the identically-priced 1.1) via the family prefix.
        assert_eq!(rates_for("muse-spark-1.2").map(|r| r.input), Some(1.25));
        assert_eq!(rates_for("muse-spark-1.2").map(|r| r.output), Some(4.25));
        assert_eq!(rates_for("meta/muse-spark-1.2").map(|r| r.cache_read), Some(0.15));
        assert_eq!(rates_for("muse-spark-1.1").map(|r| r.input), Some(1.25));
    }

    #[test]
    fn kimi_prices() {
        // Kimi K3 has its own $3/$15 rates, distinct from the K2.x catch-all.
        assert_eq!(rates_for("kimi-k3").map(|r| r.input), Some(3.0));
        assert_eq!(rates_for("kimi-k3").map(|r| r.output), Some(15.0));
        assert_eq!(rates_for("kimi-k3").map(|r| r.cache_read), Some(0.3));
        assert_eq!(rates_for("moonshot/kimi-k3-250824").map(|r| r.input), Some(3.0));
        // K2.7 Code is a coding model at $0.95/$4, far below the K2.x catch-all.
        assert_eq!(rates_for("kimi-k2.7-code").map(|r| r.input), Some(0.95));
        assert_eq!(rates_for("kimi-k2.7-code").map(|r| r.output), Some(4.0));
        assert_eq!(rates_for("kimi-k2.7-code").map(|r| r.cache_read), Some(0.19));
        // K2.6 / K2.5 match their own entries; unknown K2.x variants fall back.
        assert_eq!(rates_for("kimi-k2.6").map(|r| r.input), Some(0.95));
        assert_eq!(rates_for("kimi-k2.5").map(|r| r.input), Some(0.6));
        assert_eq!(rates_for("kimi-k2.9").map(|r| r.input), Some(2.78));
    }

    #[test]
    fn fingerprint_changes_with_the_table() {
        let a = pricing_fingerprint();
        assert_ne!(a, 0);
        // deterministic within a build
        assert_eq!(a, pricing_fingerprint());
    }
}
