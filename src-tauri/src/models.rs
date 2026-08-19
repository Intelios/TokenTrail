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
