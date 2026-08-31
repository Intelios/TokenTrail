pub mod antigravity;
pub mod claude_code;
pub mod codex;
pub mod gemini;
pub mod opencode;
pub mod wackchatter;
pub mod zcode;

use crate::models::{IngestStats, Source, SourceStatus};
use crate::store::Store;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

pub use crate::aggregate::parse_ts_ms;

pub fn sync_all(store: &Store, home: &Path) -> Vec<IngestStats> {
    type CollectFn = fn(&Store, &Path) -> Result<usize, String>;
    let runs: Vec<(&str, CollectFn)> = vec![
        ("zcode", zcode::collect),
        ("claude_code", claude_code::collect),
        ("codex", codex::collect),
        ("opencode", opencode::collect),
        ("gemini", gemini::collect),
        ("antigravity", antigravity::collect),
        ("wackchatter", wackchatter::collect),
    ];
    runs.into_iter()
        .map(|(name, f)| match f(store, home) {
            Ok(n) => IngestStats { source: name.into(), processed: n, error: None },
            Err(e) => IngestStats { source: name.into(), processed: 0, error: Some(e) },
        })
        .collect()
}

pub fn source_status(home: &Path) -> Vec<SourceStatus> {
    let paths: Vec<(Source, PathBuf)> = vec![
        (Source::Zcode, home.join(".zcode/cli/db/db.sqlite")),
        (Source::ClaudeCode, home.join(".claude/projects")),
        (Source::Codex, home.join(".codex/sessions")),
        (Source::Opencode, home.join(".local/share/opencode")),
        (Source::Gemini, home.join(".gemini/tmp")),
        (Source::Antigravity, home.join(".gemini/antigravity/conversations")),
        // The log, not the library: the library moves, and this path never does.
        (Source::WackChatter, home.join(".wackchatter/usage.jsonl")),
    ];
    paths
        .into_iter()
        .map(|(s, p)| SourceStatus {
            source: s.as_str().into(),
            display: s.display().into(),
            path: p.display().to_string(),
            found: p.exists(),
        })
        .collect()
}

/// Read only the complete new lines appended since `offset`.
/// Returns (text, new_offset); new_offset only ever advances past a '\n',
/// so a half-written tail line is retried on the next poll.
pub fn read_tail(path: &Path, offset: u64) -> std::io::Result<(String, u64)> {
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    if len < offset {
        // file was truncated/rotated: start over next poll
        return Ok((String::new(), 0));
    }
    if len == offset {
        return Ok((String::new(), offset));
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut buf = String::new();
    (&mut file).take(len - offset).read_to_string(&mut buf)?;
    match buf.rfind('\n') {
        Some(i) => Ok((buf[..=i].to_string(), offset + (i + 1) as u64)),
        None => Ok((String::new(), offset)),
    }
}

/// Strip harness decorations and provider prefixes: "claude-opus-5[ffe]" / "anthropic/claude-sonnet-4.5".
pub fn clean_model(m: &str) -> String {
    let base = m.rsplit('/').next().unwrap_or(m);
    base.split('[').next().unwrap_or(base).to_string()
}

pub fn sorted_glob(pattern: &str) -> Result<Vec<PathBuf>, String> {
    let mut files: Vec<PathBuf> = glob::glob(pattern)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    files.sort();
    Ok(files)
}

#[cfg(test)]
pub(crate) fn test_home(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tokentrail-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(test)]
pub(crate) fn test_store(tag: &str) -> Store {
    Store::open(&test_home(tag).join("usage.db")).unwrap()
}

#[cfg(test)]
pub(crate) fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures").join(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_model_decorations_and_provider_prefixes() {
        assert_eq!(clean_model("claude-opus-5"), "claude-opus-5");
        assert_eq!(clean_model("claude-opus-5[ffe]"), "claude-opus-5");
        assert_eq!(clean_model("anthropic/claude-sonnet-4.5"), "claude-sonnet-4.5");
        assert_eq!(clean_model("bedrock/claude-3-5-sonnet"), "claude-3-5-sonnet");
        assert_eq!(clean_model("provider/model[extra]"), "model");
        assert_eq!(clean_model("deepseek/deepseek-chat"), "deepseek-chat");
    }
}
