use std::collections::HashMap;
use std::sync::OnceLock;

use crate::models::UsageEvent;

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

/// Strip harness decorations: "claude-opus-5[ffe]" and "model/variant" both
/// reduce to their family-matchable base.
pub fn normalize_model(model: &str) -> String {
    let base = model.split('/').next().unwrap_or(model);
    let base = base.split('[').next().unwrap_or(base);
    base.trim().to_ascii_lowercase()
}

fn rates_for(model: &str) -> Option<Rates> {
    let t = table();
    let m = normalize_model(model);
    if m.is_empty() {
        return None;
    }
    if let Some(r) = t.models.get(&m) {
        return Some(*r);
    }
    // families are ordered longest-prefix-first in pricing.json
    t.families
        .iter()
        .find(|f| m.starts_with(&f.prefix))
        .map(|f| f.rates)
}

pub fn cost_usd(model: Option<&str>, e: &UsageEvent) -> Option<f64> {
    let r = rates_for(model?)?;
    Some(
        (e.input_tokens as f64 * r.input
            + e.cache_write_tokens as f64 * r.cache_write
            + e.cache_read_tokens as f64 * r.cache_read
            + e.output_tokens as f64 * r.output)
            / 1_000_000.0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Source;

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
    fn computes_cost() {
        // 1M input + 1M output on a $3/$15 model = $18
        let e = event(Some("claude-sonnet-4.5"), 1_000_000, 0, 0, 1_000_000);
        let cost = cost_usd(e.model.as_deref(), &e).unwrap();
        assert!((cost - 18.0).abs() < 1e-9);
        assert!(cost_usd(None, &e).is_none());
    }
}
