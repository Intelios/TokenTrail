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
    Devin,
    WackChatter,
    WackCode,
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
            Source::Devin => "devin",
            Source::WackChatter => "wackchatter",
            Source::WackCode => "wackcode",
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
            Source::Devin => "Devin CLI",
            Source::WackChatter => "WackChatter",
            Source::WackCode => "WackCode",
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
            "devin" => Some(Source::Devin),
            "wackchatter" => Some(Source::WackChatter),
            "wackcode" => Some(Source::WackCode),
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
    /// The provider's display name at record time (WackCode-only: custom
    /// connections have uuid ids, so the ledger also writes what the user
    /// typed). NULL for every other source and for old WackCode rows.
    pub provider_name: Option<String>,
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
    pub purpose: Option<String>,
    pub outcome: Option<String>,
    pub workspace: Option<String>,
    pub subagent_id: Option<String>,
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

/// A user-chosen chart color for a project, keyed by the project path exactly as
/// `usage_event.project` stores it. `color` is always lowercase `#rrggbb`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectColor {
    pub project: String,
    pub color: String,
}

/// A folder → project mapping: events recorded under `alias` count as `canonical`.
/// Filled automatically from the nearest enclosing git root, and adjustable by hand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectAlias {
    pub alias: String,
    pub canonical: String,
}
