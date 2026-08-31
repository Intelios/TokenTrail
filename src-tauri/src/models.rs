use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Zcode,
    ClaudeCode,
    Codex,
    Opencode,
    Gemini,
    Antigravity,
    WackChatter,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Zcode => "zcode",
            Source::ClaudeCode => "claude_code",
            Source::Codex => "codex",
            Source::Opencode => "opencode",
            Source::Gemini => "gemini",
            Source::Antigravity => "antigravity",
            Source::WackChatter => "wackchatter",
        }
    }

    pub fn display(self) -> &'static str {
        match self {
            Source::Zcode => "ZCode",
            Source::ClaudeCode => "Claude Code",
            Source::Codex => "Codex",
            Source::Opencode => "OpenCode",
            Source::Gemini => "Gemini CLI",
            Source::Antigravity => "Antigravity",
            Source::WackChatter => "WackChatter",
        }
    }

    /// Parse a stored `source` column value back into a `Source`.
    pub fn from_str(s: &str) -> Option<Source> {
        match s {
            "zcode" => Some(Source::Zcode),
            "claude_code" => Some(Source::ClaudeCode),
            "codex" => Some(Source::Codex),
            "opencode" => Some(Source::Opencode),
            "gemini" => Some(Source::Gemini),
            "antigravity" => Some(Source::Antigravity),
            "wackchatter" => Some(Source::WackChatter),
            _ => None,
        }
    }
}

/// One model request, normalized across harnesses.
#[derive(Debug, Clone)]
pub struct UsageEvent {
    pub source: Source,
    pub source_event_id: String,
    /// epoch milliseconds
    pub ts: i64,
    pub session_id: Option<String>,
    pub project: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: Option<i64>,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub duration_ms: Option<i64>,
    pub ttft_ms: Option<i64>,
    pub is_subagent: bool,
    /// The token counts are the app's own guess, not the provider's report.
    ///
    /// Harnesses all report real usage, so this is false for every one of them. It exists
    /// for sources that only sometimes get a number back — a count that was estimated and
    /// a count that was billed are different claims, and merging them would quietly turn
    /// one into the other.
    pub estimated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct IngestStats {
    pub source: String,
    pub processed: usize,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceStatus {
    pub source: String,
    pub display: String,
    pub path: String,
    pub found: bool,
}

/// A user-declared alias: events recorded as `alias` are displayed as `canonical`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelAlias {
    pub alias: String,
    pub canonical: String,
}
