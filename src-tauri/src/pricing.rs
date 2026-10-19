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
    /// Current list price, open-ended.
    rates: Rates,
    /// Earlier prices of a floating API name (`deepseek-flash` moves to each
    /// new release, and its price moves with it). When a floating name's price
    /// changes, the old `rates` move here with `until` set to the release date
    /// rather than being overwritten, so history keeps the price it ran at.
    #[serde(default)]
    history: Vec<Dated>,
}

#[derive(Debug, serde::Deserialize)]
struct Dated {
    /// Exclusive end, UTC: `YYYY-MM-DD` (midnight) or RFC 3339.
    until: String,
    rates: Rates,
    #[serde(skip)]
    until_ms: i64,
}

impl Family {
    fn rates_at(&self, ts_ms: i64) -> Rates {
        self.history
            .iter()
            .find(|d| ts_ms < d.until_ms)
            .map_or(self.rates, |d| d.rates)
    }
}

/// Epoch ms of a pricing `until` date; `None` when it is neither form.
fn parse_until(s: &str) -> Option<i64> {
    use time::format_description::well_known::Rfc3339;
    if let Ok(t) = time::OffsetDateTime::parse(s, &Rfc3339) {
        return Some((t.unix_timestamp_nanos() / 1_000_000) as i64);
    }
    let day = time::macros::format_description!("[year]-[month]-[day]");
    let d = time::Date::parse(s, &day).ok()?;
    Some(d.midnight().assume_utc().unix_timestamp() * 1000)
}

fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t: Table =
            serde_json::from_str(PRICING_JSON).expect("embedded pricing.json is valid");
        for f in &mut t.families {
            for d in &mut f.history {
                d.until_ms = parse_until(&d.until)
                    .unwrap_or_else(|| panic!("pricing.json: bad until {:?} for {}", d.until, f.prefix));
            }
            f.history.sort_by_key(|d| d.until_ms);
        }
        t
    })
}

/// Strip harness decorations and provider prefixes: "claude-opus-5[ffe]" and
/// "anthropic/claude-sonnet-4.5" both reduce to their family-matchable base
/// (the last path segment, minus any "[suffix]" decoration).
pub fn normalize_model(model: &str) -> String {
    let base = model.rsplit('/').next().unwrap_or(model);
    let base = base.split('[').next().unwrap_or(base);
    base.trim().to_ascii_lowercase()
}

/// Current rates for `model`.
#[cfg(test)]
fn rates_for(model: &str) -> Option<Rates> {
    rates_at(model, i64::MAX)
}

/// Rates `model` was listed at when an event at `ts_ms` happened.
fn rates_at(model: &str, ts_ms: i64) -> Option<Rates> {
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
    // Families prefer the provider-stripped base: it is the model name, while
    // a match on the full "provider/model" string can only come from a
    // catch-all prefix swallowing the provider ("deepseek/deepseek-flash"
    // must hit the deepseek-flash family, not the deepseek catch-all).
    let by_base: Vec<&str> = candidates.iter().rev().cloned().collect();
    for c in &by_base {
        if let Some(f) = t.families.iter().find(|f| c.starts_with(&f.prefix)) {
            return Some(f.rates_at(ts_ms));
        }
    }
    None
}

/// API-equivalent cost in USD of one model request at `ts_ms` (epoch ms), from
/// the bundled list prices. Returns `None` when the model is unknown or missing.
#[allow(clippy::too_many_arguments)]
pub fn cost_for(
    source: Source,
    model: Option<&str>,
    ts_ms: i64,
    input_tokens: i64,
    output_tokens: i64,
    reasoning_tokens: Option<i64>,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
) -> Option<f64> {
    let r = rates_at(model?, ts_ms)?;
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
        e.ts,
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
            provider_name: None,
            model: model.map(String::from),
            input_tokens: input,
            output_tokens: out,
            reasoning_tokens: None,
            cache_read_tokens: cr,
            cache_write_tokens: cw,
            duration_ms: None,
            ttft_ms: None,
            is_subagent: false,
            estimated: false,
            purpose: None, outcome: None, workspace: None, subagent_id: None, mode: None,
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
    fn mimo_v26_prices() {
        // V2.6 keeps V2.5's list rates: Pro is $0.435/$0.87 with $0.0036
        // cached input, Flash is $0.14/$0.28 with $0.0028 cached input;
        // cache writes are limited-time free and bill at the input rate.
        assert_eq!(rates_for("mimo-v2.6-pro").map(|r| r.input), Some(0.435));
        assert_eq!(rates_for("mimo-v2.6-pro").map(|r| r.output), Some(0.87));
        assert_eq!(rates_for("mimo-v2.6-pro").map(|r| r.cache_read), Some(0.0036));
        assert_eq!(rates_for("mimo-v2.6-pro").map(|r| r.cache_write), Some(0.435));
        assert_eq!(rates_for("mimo-v2.6-flash").map(|r| r.input), Some(0.14));
        assert_eq!(rates_for("mimo-v2.6-flash").map(|r| r.output), Some(0.28));
        assert_eq!(rates_for("mimo-v2.6-flash").map(|r| r.cache_read), Some(0.0028));
        assert_eq!(rates_for("mimo-v2.6-flash").map(|r| r.cache_write), Some(0.14));
        // Provider-prefixed spellings resolve through the family.
        assert_eq!(rates_for("xiaomi/mimo-v2.6-pro").map(|r| r.input), Some(0.435));
        // Older MiMo names keep their own entries and the catch-all.
        assert_eq!(rates_for("mimo-v2.5-pro").map(|r| r.input), Some(0.435));
        assert_eq!(rates_for("mimo-v2.5").map(|r| r.input), Some(0.112));
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
    fn gemini_4_argon_prices() {
        // Gemini 4 Argon (the only Gemini 4 model) lists at $2/$10 with $0.10
        // implicitly-cached input through November 30, 2026, doubling to
        // $4/$20/$0.20 from December 1, 2026; the table carries the current
        // rates. Explicit caching is unsupported and cache writes bill at
        // the uncached input rate, as for the other Gemini families.
        assert_eq!(rates_for("gemini-4-argon").map(|r| r.input), Some(2.0));
        assert_eq!(rates_for("gemini-4-argon").map(|r| r.output), Some(10.0));
        assert_eq!(rates_for("gemini-4-argon").map(|r| r.cache_read), Some(0.1));
        assert_eq!(rates_for("gemini-4-argon").map(|r| r.cache_write), Some(2.0));
        // Dated snapshots and provider-prefixed spellings resolve via the family.
        assert_eq!(rates_for("gemini-4-argon-2026-10-13").map(|r| r.output), Some(10.0));
        assert_eq!(rates_for("google/gemini-4-argon").map(|r| r.input), Some(2.0));
        // The entry outranks the generic gemini catch-all ($0.30/$2.50),
        // and older Gemini generations keep their own rates.
        assert_eq!(rates_for("gemini-3.7-flash").map(|r| r.input), Some(0.75));
        assert_eq!(rates_for("gemini-2.5-pro").map(|r| r.input), Some(1.25));
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
    fn glm_flash_prices() {
        // GLM-5.3-Flash lists at $0.15/$0.50, far below the GLM-5.x catch-all.
        assert_eq!(rates_for("glm-5.3-flash").map(|r| r.input), Some(0.15));
        assert_eq!(rates_for("glm-5.3-flash").map(|r| r.output), Some(0.5));
        assert_eq!(rates_for("glm-5.3-flash").map(|r| r.cache_read), Some(0.03));
        assert_eq!(rates_for("zai/glm-5.3-flash").map(|r| r.input), Some(0.15));
        // Other GLM variants keep the catch-all rate.
        assert_eq!(rates_for("glm-5.3").map(|r| r.input), Some(1.4));
    }

    #[test]
    fn longcat_prices() {
        // LongCat-2.5-Preview (Meituan, limited-time rates) lists at
        // $0.30/$1.20 with $0.006 cached input; cache writes are undocumented
        // and bill at the uncached input rate.
        assert_eq!(rates_for("longcat-2.5-preview").map(|r| r.input), Some(0.3));
        assert_eq!(rates_for("longcat-2.5-preview").map(|r| r.output), Some(1.2));
        assert_eq!(rates_for("longcat-2.5-preview").map(|r| r.cache_read), Some(0.006));
        assert_eq!(rates_for("longcat-2.5-preview").map(|r| r.cache_write), Some(0.3));
        assert_eq!(rates_for("meituan/longcat-2.5-preview").map(|r| r.output), Some(1.2));
        // LongCat-2.0 (Meituan) lists at $0.75/$2.95 with $0.015 cached input;
        // the longcat-2.5 prefix must not swallow it.
        assert_eq!(rates_for("longcat-2.0").map(|r| r.input), Some(0.75));
        assert_eq!(rates_for("longcat-2.0").map(|r| r.output), Some(2.95));
        assert_eq!(rates_for("longcat-2.0").map(|r| r.cache_read), Some(0.015));
        assert_eq!(rates_for("longcat-2.0").map(|r| r.cache_write), Some(0.75));
        assert_eq!(rates_for("meituan/longcat-2.0").map(|r| r.output), Some(2.95));
    }

    #[test]
    fn hy4_prices() {
        // Tencent Hunyuan Hy4 preview: $0.834/$2.501 with $0.042 cached input;
        // cache writes are undocumented and bill at the uncached input rate.
        assert_eq!(rates_for("hy4-preview").map(|r| r.input), Some(0.834));
        assert_eq!(rates_for("hy4-preview").map(|r| r.output), Some(2.501));
        assert_eq!(rates_for("hy4-preview").map(|r| r.cache_read), Some(0.042));
        assert_eq!(rates_for("hy4-preview").map(|r| r.cache_write), Some(0.834));
        // Both provider-stripped and Hunyuan-branded spellings resolve.
        assert_eq!(rates_for("tencent/hy4-preview").map(|r| r.input), Some(0.834));
        assert_eq!(rates_for("hunyuan-hy4-preview").map(|r| r.input), Some(0.834));
    }

    #[test]
    fn gpt_6_astra_prices() {
        // GPT-6 Astra lists at $10/$50, well above the generic gpt catch-all.
        assert_eq!(rates_for("gpt-6-astra").map(|r| r.input), Some(10.0));
        assert_eq!(rates_for("gpt-6-astra").map(|r| r.output), Some(50.0));
        assert_eq!(rates_for("gpt-6-astra").map(|r| r.cache_read), Some(1.0));
        assert_eq!(rates_for("gpt-6-astra").map(|r| r.cache_write), Some(12.5));
        // Dated snapshots and provider-prefixed spellings resolve through the family.
        assert_eq!(rates_for("gpt-6-astra-2026-09-04").map(|r| r.output), Some(50.0));
        assert_eq!(rates_for("openai/gpt-6-astra").map(|r| r.input), Some(10.0));
    }

    #[test]
    fn gpt_6_sol_luna_prices() {
        // GPT-6 Sol (launched 2026-09-22) lists at $2/$10 with $0.20 cached
        // input and a 1.25x cache write ($2.50); GPT-6 Luna is the small
        // sibling at $0.10/$0.50 with $0.01 cached input and a $0.125 write.
        assert_eq!(rates_for("gpt-6-sol").map(|r| r.input), Some(2.0));
        assert_eq!(rates_for("gpt-6-sol").map(|r| r.output), Some(10.0));
        assert_eq!(rates_for("gpt-6-sol").map(|r| r.cache_read), Some(0.2));
        assert_eq!(rates_for("gpt-6-sol").map(|r| r.cache_write), Some(2.5));
        assert_eq!(rates_for("gpt-6-luna").map(|r| r.input), Some(0.1));
        assert_eq!(rates_for("gpt-6-luna").map(|r| r.output), Some(0.5));
        assert_eq!(rates_for("gpt-6-luna").map(|r| r.cache_read), Some(0.01));
        assert_eq!(rates_for("gpt-6-luna").map(|r| r.cache_write), Some(0.125));
        // Dated snapshots, provider prefixes and "model/variant" spellings
        // resolve through the family; GPT-6 Astra keeps its own $10/$50 rates.
        assert_eq!(rates_for("gpt-6-sol-2026-09-22").map(|r| r.output), Some(10.0));
        assert_eq!(rates_for("openai/gpt-6-luna").map(|r| r.input), Some(0.1));
        assert_eq!(rates_for("gpt-6-sol/long-context").map(|r| r.input), Some(2.0));
        assert_eq!(rates_for("gpt-6-astra").map(|r| r.output), Some(50.0));
    }

    #[test]
    fn sonnet_55_prices() {
        // Claude Sonnet 5.5 keeps Sonnet 5's $2/$10 pricing with the
        // standard 10% cache hit ($0.20) and 1.25x five-minute cache write
        // ($2.50) — the 0.025x/0.05x cache-hit discounts are limited to
        // Fable/Mythos 5.1 and Opus 5.5. Both the hyphenated API id and the
        // dot spelling resolve; their prefixes sit above claude-sonnet-5,
        // which would otherwise swallow them.
        assert_eq!(rates_for("claude-sonnet-5-5").map(|r| r.input), Some(2.0));
        assert_eq!(rates_for("claude-sonnet-5-5").map(|r| r.output), Some(10.0));
        assert_eq!(rates_for("claude-sonnet-5-5").map(|r| r.cache_read), Some(0.2));
        assert_eq!(rates_for("claude-sonnet-5-5").map(|r| r.cache_write), Some(2.5));
        assert_eq!(rates_for("claude-sonnet-5.5").map(|r| r.input), Some(2.0));
        assert_eq!(rates_for("claude-sonnet-5.5[ffe]").map(|r| r.output), Some(10.0));
        assert_eq!(rates_for("claude-sonnet-5.5-20261026").map(|r| r.output), Some(10.0));
        assert_eq!(rates_for("claude-sonnet-5-5-20261026").map(|r| r.cache_read), Some(0.2));
        assert_eq!(rates_for("anthropic/claude-sonnet-5.5").map(|r| r.input), Some(2.0));
        // Plain Sonnet 5 carries the same list rates; Sonnet 4.x keeps $3/$15.
        assert_eq!(rates_for("claude-sonnet-5").map(|r| r.input), Some(2.0));
        assert_eq!(rates_for("claude-sonnet-4.6").map(|r| r.input), Some(3.0));
    }

    #[test]
    fn deepseek_v41_flash_prices() {
        // V4.1 Flash lists at $0.15/$0.60 off-peak and 2x at peak; the table
        // carries peak rates like the other deepseek entries. "deepseek-flash"
        // is the official model ID, "deepseek-v4.1-flash" the version-style
        // spelling, and the retired deepseek-v4-flash / -vision-exp names now
        // route to V4.1 Flash at the same price.
        assert_eq!(rates_for("deepseek-flash").map(|r| r.input), Some(0.3));
        assert_eq!(rates_for("deepseek-flash").map(|r| r.output), Some(1.2));
        assert_eq!(rates_for("deepseek-flash").map(|r| r.cache_read), Some(0.006));
        assert_eq!(rates_for("deepseek-flash").map(|r| r.cache_write), Some(0.3));
        assert_eq!(rates_for("deepseek-v4.1-flash:0908-cloud").map(|r| r.input), Some(0.3));
        assert_eq!(rates_for("deepseek/deepseek-flash").map(|r| r.output), Some(1.2));
        assert_eq!(rates_for("deepseek-v4-flash:0731-cloud").map(|r| r.input), Some(0.3));
        assert_eq!(rates_for("deepseek-v4-flash-vision-exp").map(|r| r.output), Some(1.2));
        // The $0 free variant keeps its exact-model override over the family.
        assert_eq!(rates_for("deepseek-v4-flash-free").map(|r| r.input), Some(0.0));
        // Unrelated deepseek spellings keep the catch-all rate.
        assert_eq!(rates_for("deepseek-chat").map(|r| r.input), Some(0.44));
    }

    #[test]
    fn swe_2_prices() {
        // Cognition's SWE-2 (all reasoning-effort variants: swe-2-high/max/
        // medium) lists at $3/$15 with $0.30 cached input; Devin reports no
        // cache-creation tokens, and Cognition lists no cache-write rate.
        assert_eq!(rates_for("swe-2-high").map(|r| r.input), Some(3.0));
        assert_eq!(rates_for("swe-2-max").map(|r| r.output), Some(15.0));
        assert_eq!(rates_for("swe-2-medium").map(|r| r.cache_read), Some(0.3));
        assert_eq!(rates_for("swe-2-medium").map(|r| r.cache_write), Some(0.0));
        // Reasoning-effort suffixes and provider-prefixed spellings resolve.
        assert_eq!(rates_for("swe-2-high-2026-09-16").map(|r| r.input), Some(3.0));
        assert_eq!(rates_for("windsurf/swe-2-max").map(|r| r.output), Some(15.0));
        // Older SWE-1.x generations don't adopt the SWE-2 rates.
        assert_eq!(rates_for("swe-1-6-slow").map(|r| r.input), Some(0.5));
        assert_eq!(rates_for("swe-1-7").map(|r| r.input), Some(0.5));
    }

    #[test]
    fn swe_1_x_prices() {
        // SWE-1.6 (base / fast / slow effort variants) lists at $0.50/$2.50
        // with $0.20 cached input and no cache-write rate.
        assert_eq!(rates_for("swe-1-6").map(|r| r.input), Some(0.5));
        assert_eq!(rates_for("swe-1-6-fast").map(|r| r.output), Some(2.5));
        assert_eq!(rates_for("swe-1-6-slow").map(|r| r.cache_read), Some(0.2));
        assert_eq!(rates_for("swe-1-6-slow").map(|r| r.cache_write), Some(0.0));
        assert_eq!(rates_for("windsurf/swe-1-6-fast").map(|r| r.input), Some(0.5));
        // SWE-1.7 base carries the same rates as 1.6.
        assert_eq!(rates_for("swe-1-7").map(|r| r.input), Some(0.5));
        assert_eq!(rates_for("swe-1-7-medium").map(|r| r.output), Some(2.5));
        // SWE-1.7 Lightning (Cerebras-served) is far pricier: $2.50/$12.50
        // with $1.00 cached input; its prefix must outrank plain swe-1-7.
        assert_eq!(rates_for("swe-1-7-lightning").map(|r| r.input), Some(2.5));
        assert_eq!(rates_for("swe-1-7-lightning").map(|r| r.output), Some(12.5));
        assert_eq!(rates_for("swe-1-7-lightning-medium").map(|r| r.cache_read), Some(1.0));
        assert_eq!(rates_for("swe-1-7-lightning-2026-08-01").map(|r| r.output), Some(12.5));
        assert_eq!(rates_for("windsurf/swe-1-7-lightning").map(|r| r.input), Some(2.5));
    }

    #[test]
    fn gpt_6_1_sol_prices() {
        // GPT-6.1 Sol keeps GPT-6 Sol's $2/$10 standard short-context list
        // rates and 1.25x cache write ($2.50), but cache hits halved to
        // $0.10 (vs GPT-6 Sol's $0.20). The table carries standard
        // short-context rates throughout; long-context (>272K) tiers are
        // ignored, as for the other entries.
        assert_eq!(rates_for("gpt-6.1-sol").map(|r| r.input), Some(2.0));
        assert_eq!(rates_for("gpt-6.1-sol").map(|r| r.output), Some(10.0));
        assert_eq!(rates_for("gpt-6.1-sol").map(|r| r.cache_read), Some(0.1));
        assert_eq!(rates_for("gpt-6.1-sol").map(|r| r.cache_write), Some(2.5));
        // Dated snapshots, provider prefixes and "model/variant" spellings
        // resolve through the family.
        assert_eq!(rates_for("gpt-6.1-sol-2026-10-26").map(|r| r.output), Some(10.0));
        assert_eq!(rates_for("openai/gpt-6.1-sol").map(|r| r.cache_read), Some(0.1));
        assert_eq!(rates_for("gpt-6.1-sol/long-context").map(|r| r.input), Some(2.0));
        // The dot in "6.1" means the gpt-6-sol prefix can't swallow it, and
        // plain GPT-6 Sol keeps its own (pricier) $0.20 cache-hit rate.
        assert_eq!(rates_for("gpt-6-sol").map(|r| r.cache_read), Some(0.2));
        assert_eq!(rates_for("gpt-6-astra").map(|r| r.output), Some(50.0));
    }

    #[test]
    fn opus_55_prices() {
        // Opus 5.5 (launched 2026-09-22) dropped to $4/$20 with a 5% cache hit
        // ($0.20) and Anthropic's standard 1.25x cache write ($5). Both the
        // hyphenated API id and the dot spelling resolve.
        assert_eq!(rates_for("claude-opus-5-5").map(|r| r.input), Some(4.0));
        assert_eq!(rates_for("claude-opus-5-5").map(|r| r.output), Some(20.0));
        assert_eq!(rates_for("claude-opus-5-5").map(|r| r.cache_read), Some(0.2));
        assert_eq!(rates_for("claude-opus-5-5").map(|r| r.cache_write), Some(5.0));
        assert_eq!(rates_for("claude-opus-5.5").map(|r| r.input), Some(4.0));
        assert_eq!(rates_for("claude-opus-5.5[ffe]").map(|r| r.output), Some(20.0));
        assert_eq!(rates_for("claude-opus-5-5-20260922").map(|r| r.output), Some(20.0));
        assert_eq!(rates_for("anthropic/claude-opus-5.5").map(|r| r.input), Some(4.0));
        // Plain Opus 5 keeps the $5/$25 catch-all; Opus 4.x keeps $15/$75.
        assert_eq!(rates_for("claude-opus-5").map(|r| r.input), Some(5.0));
        assert_eq!(rates_for("claude-opus-5").map(|r| r.output), Some(25.0));
        assert_eq!(rates_for("claude-opus-4-8").map(|r| r.input), Some(15.0));
    }

    #[test]
    fn grok_47_prices() {
        // Grok 4.7 lists at $2/$6 with $0.50 cached input; no cache-write
        // rate is published, so writes bill at the uncached input rate. The
        // >200K long-context tier ($4/$12/$1) is ignored, as for other
        // entries. Sibling Grok generations keep no pricing of their own.
        assert_eq!(rates_for("grok-4.7").map(|r| r.input), Some(2.0));
        assert_eq!(rates_for("grok-4.7").map(|r| r.output), Some(6.0));
        assert_eq!(rates_for("grok-4.7").map(|r| r.cache_read), Some(0.5));
        assert_eq!(rates_for("grok-4.7").map(|r| r.cache_write), Some(2.0));
        // Provider-prefixed spellings and dated snapshots resolve via the family.
        assert_eq!(rates_for("x-ai/grok-4.7").map(|r| r.input), Some(2.0));
        assert_eq!(rates_for("xai/grok-4.7").map(|r| r.output), Some(6.0));
        assert_eq!(rates_for("grok-4.7-20261112").map(|r| r.cache_read), Some(0.5));
        // The prefix does not swallow earlier Grok generations.
        assert!(rates_for("grok-4.6").is_none());
        assert!(rates_for("grok-4.20").is_none());
    }

    #[test]
    fn step_5_preview_prices() {
        // StepFun Step 5 Preview lists at $1/$2.70 with $0.05 cached input;
        // cache writes are undocumented and bill at the uncached input rate.
        assert_eq!(rates_for("step-5-preview").map(|r| r.input), Some(1.0));
        assert_eq!(rates_for("step-5-preview").map(|r| r.output), Some(2.7));
        assert_eq!(rates_for("step-5-preview").map(|r| r.cache_read), Some(0.05));
        assert_eq!(rates_for("step-5-preview").map(|r| r.cache_write), Some(1.0));
        // The provider-prefixed OpenRouter spelling resolves via the family.
        assert_eq!(rates_for("stepfun/step-5-preview").map(|r| r.output), Some(2.7));
        // A speculative step-5 catch-all (preview rates until StepFun
        // publishes launch pricing) covers the out-of-preview spelling and
        // variants; it sits after step-5-preview so the preview keeps its
        // own entry if the rates later diverge.
        assert_eq!(rates_for("step-5").map(|r| r.input), Some(1.0));
        assert_eq!(rates_for("step-5").map(|r| r.output), Some(2.7));
        assert_eq!(rates_for("step-5-max").map(|r| r.cache_read), Some(0.05));
        assert_eq!(rates_for("stepfun/step-5").map(|r| r.output), Some(2.7));
        // Older Step generations keep no pricing of their own.
        assert!(rates_for("step-3.7-flash").is_none());
    }

    #[test]
    fn haiku_55_prices() {
        // Claude Haiku 5.5 lists at $0.10/$0.50 with the standard 10%
        // cache hit ($0.01) and 1.25x five-minute cache write ($0.125) —
        // the 0.05x cache-hit discounts are limited to Fable/Mythos 5.1
        // and Opus/Sonnet 5.5. Both the hyphenated API id and the dot
        // spelling resolve; their prefixes sit above claude-haiku, which
        // would otherwise swallow them at the old $1/$5 Haiku 4.5 rate.
        // The >100K-token long-context tier ($0.50/$2.50) is ignored, as
        // for the other entries with long-context tiers.
        assert_eq!(rates_for("claude-haiku-5-5").map(|r| r.input), Some(0.1));
        assert_eq!(rates_for("claude-haiku-5-5").map(|r| r.output), Some(0.5));
        assert_eq!(rates_for("claude-haiku-5-5").map(|r| r.cache_read), Some(0.01));
        assert_eq!(rates_for("claude-haiku-5-5").map(|r| r.cache_write), Some(0.125));
        assert_eq!(rates_for("claude-haiku-5.5").map(|r| r.input), Some(0.1));
        assert_eq!(rates_for("claude-haiku-5.5[ffe]").map(|r| r.output), Some(0.5));
        assert_eq!(rates_for("claude-haiku-5.5-20261007").map(|r| r.output), Some(0.5));
        assert_eq!(rates_for("claude-haiku-5-5-20261007").map(|r| r.cache_read), Some(0.01));
        assert_eq!(rates_for("anthropic/claude-haiku-5.5").map(|r| r.input), Some(0.1));
        // Older Haiku generations keep the $1/$5 catch-all rate.
        assert_eq!(rates_for("claude-haiku-4.5").map(|r| r.input), Some(1.0));
        assert_eq!(rates_for("claude-haiku-4.5").map(|r| r.output), Some(5.0));
    }

    #[test]
    fn muse_wonder_prices() {
        // Muse Wonder (`muse-wonder-1.0` on Meta's Model API) lists at
        // $4/$20 with $0.20 cached input; no cache-write rate is published,
        // so writes bill at the uncached input rate, as for the other
        // entries with undocumented cache writes.
        assert_eq!(rates_for("muse-wonder").map(|r| r.input), Some(4.0));
        assert_eq!(rates_for("muse-wonder").map(|r| r.output), Some(20.0));
        assert_eq!(rates_for("muse-wonder").map(|r| r.cache_read), Some(0.2));
        assert_eq!(rates_for("muse-wonder").map(|r| r.cache_write), Some(4.0));
        // The versioned API id and provider-prefixed spellings resolve.
        assert_eq!(rates_for("muse-wonder-1.0").map(|r| r.input), Some(4.0));
        assert_eq!(rates_for("meta/muse-wonder").map(|r| r.output), Some(20.0));
        assert_eq!(rates_for("meta/muse-wonder-1.0").map(|r| r.cache_read), Some(0.2));
        // Muse Spark keeps its own $1.25/$4.25 rates.
        assert_eq!(rates_for("muse-spark-1.2").map(|r| r.input), Some(1.25));
    }

    #[test]
    fn beam_1_prices() {
        // Reflection Beam 1.0 (501B total / 23B active MoE) lists at
        // $1/$3 with $0.10 cached input (the full-precision reference
        // rate; quantized/discounted host endpoints go lower). No
        // cache-write rate is published, so writes bill at the uncached
        // input rate, as for the other entries with undocumented writes.
        assert_eq!(rates_for("beam-1.0-501b-a23b").map(|r| r.input), Some(1.0));
        assert_eq!(rates_for("beam-1.0-501b-a23b").map(|r| r.output), Some(3.0));
        assert_eq!(rates_for("beam-1.0-501b-a23b").map(|r| r.cache_read), Some(0.1));
        assert_eq!(rates_for("beam-1.0-501b-a23b").map(|r| r.cache_write), Some(1.0));
        // Provider-prefixed spellings resolve via the family.
        assert_eq!(rates_for("reflection/beam-1.0-501b-a23b").map(|r| r.output), Some(3.0));
        assert_eq!(rates_for("together_ai/reflection/beam-1.0-501b-a23b").map(|r| r.input), Some(1.0));
    }

    #[test]
    fn minimax_m31_flash_prices() {
        // MiniMax M3.1 Flash (launched 2026-10-19) lists at $0.15/$0.60 per
        // 1M tokens with $0.015 cached input (10% cache hit discount). Cache
        // writes are undocumented and bill at the uncached input rate ($0.15),
        // as for the other entries with undocumented writes.
        assert_eq!(rates_for("minimax-m3.1-flash").map(|r| r.input), Some(0.15));
        assert_eq!(rates_for("minimax-m3.1-flash").map(|r| r.output), Some(0.6));
        assert_eq!(rates_for("minimax-m3.1-flash").map(|r| r.cache_read), Some(0.015));
        assert_eq!(rates_for("minimax-m3.1-flash").map(|r| r.cache_write), Some(0.15));
        // Both dot and hyphen spellings resolve.
        assert_eq!(rates_for("minimax-m3-1-flash").map(|r| r.input), Some(0.15));
        assert_eq!(rates_for("minimax-m3-1-flash").map(|r| r.output), Some(0.6));
        assert_eq!(rates_for("minimax-m3-1-flash").map(|r| r.cache_read), Some(0.015));
        assert_eq!(rates_for("minimax-m3-1-flash").map(|r| r.cache_write), Some(0.15));
        // Dated snapshots, provider prefixes and uppercase spellings resolve.
        assert_eq!(rates_for("MiniMax-M3.1-Flash").map(|r| r.input), Some(0.15));
        assert_eq!(rates_for("minimax/minimax-m3.1-flash").map(|r| r.output), Some(0.6));
        assert_eq!(rates_for("minimax/MiniMax-M3.1-Flash").map(|r| r.cache_read), Some(0.015));
        assert_eq!(rates_for("minimax-m3.1-flash-20261019").map(|r| r.output), Some(0.6));
        // Cost calculation: 1M input ($0.15) + 1M output ($0.60) + 1M cached input ($0.015) = $0.765
        let e = event(Some("minimax-m3.1-flash"), 1_000_000, 0, 1_000_000, 1_000_000);
        let cost = cost_usd(e.model.as_deref(), &e).unwrap();
        assert!((cost - 0.765).abs() < 1e-9);
    }

    #[test]
    fn dated_family_rates_follow_event_time() {
        let r = |input| Rates { input, output: 0.0, cache_read: 0.0, cache_write: 0.0 };
        let cut = parse_until("2026-10-17").unwrap();
        let f = Family {
            prefix: "deepseek-flash".into(),
            rates: r(0.5),
            history: vec![Dated { until: "2026-10-17".into(), rates: r(0.3), until_ms: cut }],
        };
        assert_eq!(f.rates_at(cut - 1).input, 0.3);
        assert_eq!(f.rates_at(cut).input, 0.5);
        assert_eq!(f.rates_at(i64::MAX).input, 0.5);
    }

    #[test]
    fn pricing_until_dates_parse() {
        assert_eq!(parse_until("1970-01-02"), Some(86_400_000));
        assert_eq!(parse_until("1970-01-01T00:00:01Z"), Some(1000));
        assert_eq!(parse_until("Oct 17"), None);
        // Every embedded history date parses (table() panics otherwise) and is sorted.
        for f in &table().families {
            assert!(f.history.windows(2).all(|w| w[0].until_ms <= w[1].until_ms));
        }
    }

    #[test]
    fn fingerprint_changes_with_the_table() {
        let a = pricing_fingerprint();
        assert_ne!(a, 0);
        // deterministic within a build
        assert_eq!(a, pricing_fingerprint());
    }
}
