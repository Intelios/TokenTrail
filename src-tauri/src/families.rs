use crate::pricing::normalize_model;

/// Canonical family names used as both keys and display labels.
/// The ORDER here determines the prefix match priority — more-specific
/// prefixes must appear before catch-alls within the same brand.
const FAMILY_RULES: &[(&str, &str)] = &[
    // OpenAI reasoning / code models
    ("o3-mini", "GPT"),
    ("o3", "GPT"),
    ("o4", "GPT"),
    ("codex", "GPT"),
    // GPT catch-all
    ("gpt", "GPT"),
    // Claude family
    ("claude-opus-4", "Claude"),
    ("claude-opus", "Claude"),
    ("claude-fable", "Claude"),
    ("claude-sonnet", "Claude"),
    ("claude-haiku", "Claude"),
    ("claude", "Claude"),
    // Gemini
    ("gemini-3.7-flash", "Gemini"),
    ("gemini-2.5-pro", "Gemini"),
    ("gemini", "Gemini"),
    // DeepSeek
    ("deepseek-v4-pro", "DeepSeek"),
    ("deepseek", "DeepSeek"),
    // Other families
    ("kimi", "Kimi"),
    ("qwen", "Qwen"),
    ("glm", "GLM"),
    ("mimo", "MiMo"),
    // Minimax
    ("minimax", "Minimax"),
    ("abab", "Minimax"),
    // Cognition — the SWE line, usually arriving via the Devin CLI
    ("swe", "SWE"),
    // Meta — Llama weights plus the Muse line; family groups by provider
    ("meta-llama", "Meta"),
    ("llama", "Meta"),
    ("muse", "Meta"),
    ("codestral", "Mistral"),
    ("mistral", "Mistral"),
    ("grok", "Grok"),
];

/// Assign a model display name to its family.  Returns "Other" when no
/// known prefix matches.
///
/// Uses the same provider-stripping logic as `pricing::normalize_model` so
/// that "anthropic/claude-sonnet-4.5" and "claude-opus-5[ffe]" both resolve
/// to "Claude".
pub fn family_for(model: &str) -> &'static str {
    let norm = normalize_model(model);
    let base = norm.as_str();
    for (prefix, family) in FAMILY_RULES {
        if base.starts_with(prefix) {
            return family;
        }
    }
    "Other"
}

/// The canonical display spelling of a family, matched case-insensitively so a
/// URL segment (`claude`, `gpt`) resolves to the label the rest of the UI uses.
pub fn canonical_family_name(input: &str) -> Option<&'static str> {
    let want = input.trim();
    if want.eq_ignore_ascii_case("Other") {
        return Some("Other");
    }
    FAMILY_RULES
        .iter()
        .map(|(_, family)| *family)
        .find(|family| family.eq_ignore_ascii_case(want))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_variants() {
        assert_eq!(family_for("claude-opus-5"), "Claude");
        assert_eq!(family_for("claude-sonnet-4.5"), "Claude");
        assert_eq!(family_for("claude-haiku-4"), "Claude");
        assert_eq!(family_for("Claude-Sonnet-4.5[ffe]"), "Claude");
        assert_eq!(family_for("anthropic/claude-sonnet-4.5"), "Claude");
    }

    #[test]
    fn gpt_and_reasoning() {
        assert_eq!(family_for("gpt-5.6-luna"), "GPT");
        assert_eq!(family_for("gpt-5"), "GPT");
        assert_eq!(family_for("gpt-4.1"), "GPT");
        assert_eq!(family_for("o3-mini"), "GPT");
        assert_eq!(family_for("o3"), "GPT");
        assert_eq!(family_for("o4"), "GPT");
        assert_eq!(family_for("codex-auto-review"), "GPT");
    }

    #[test]
    fn other_families() {
        assert_eq!(family_for("gemini-3-pro"), "Gemini");
        assert_eq!(family_for("gemini-2.5-pro"), "Gemini");
        assert_eq!(family_for("deepseek-v4-pro:0813-cloud"), "DeepSeek");
        assert_eq!(family_for("deepseek-v4-flash:0731-cloud"), "DeepSeek");
        assert_eq!(family_for("GLM-5.3"), "GLM");
        assert_eq!(family_for("glm-5.3"), "GLM");
        assert_eq!(family_for("kimi-k2"), "Kimi");
        assert_eq!(family_for("qwen3.8-max"), "Qwen");
        assert_eq!(family_for("mimo-v2.5-pro"), "MiMo");
        assert_eq!(family_for("MiniMax-M3.1-Flash"), "Minimax");
        assert_eq!(family_for("minimax-m3.1-flash"), "Minimax");
        assert_eq!(family_for("minimax/minimax-m3.1-flash"), "Minimax");
        assert_eq!(family_for("abab6.5s"), "Minimax");
    }

    #[test]
    fn meta_mistral_grok() {
        assert_eq!(family_for("llama-4-maverick"), "Meta");
        assert_eq!(family_for("Llama-4-Scout"), "Meta");
        assert_eq!(family_for("meta-llama-3.3-70b"), "Meta");
        assert_eq!(family_for("muse-spark-1.2"), "Meta");
        assert_eq!(family_for("Muse-Spark-1.2[ffe]"), "Meta");
        assert_eq!(family_for("mistral-large-latest"), "Mistral");
        assert_eq!(family_for("codestral-latest"), "Mistral");
        assert_eq!(family_for("grok-4"), "Grok");
        assert_eq!(family_for("grok-code-fast-1"), "Grok");
    }

    #[test]
    fn cognition_swe() {
        assert_eq!(family_for("swe-2-max"), "SWE");
        assert_eq!(family_for("swe-2-high"), "SWE");
        assert_eq!(family_for("swe-1-6-slow"), "SWE");
        assert_eq!(family_for("cognition/swe-2"), "SWE");
    }

    #[test]
    fn unknown_is_other() {
        assert_eq!(family_for("totally-unknown-model"), "Other");
        assert_eq!(family_for(""), "Other");
    }

    #[test]
    fn canonical_names_resolve_case_insensitively() {
        assert_eq!(canonical_family_name("claude"), Some("Claude"));
        assert_eq!(canonical_family_name("Claude"), Some("Claude"));
        assert_eq!(canonical_family_name("GPT"), Some("GPT"));
        assert_eq!(canonical_family_name("minimax"), Some("Minimax"));
        assert_eq!(canonical_family_name(" other "), Some("Other"));
        assert_eq!(canonical_family_name("Claude Family"), None);
        assert_eq!(canonical_family_name(""), None);
    }
}
