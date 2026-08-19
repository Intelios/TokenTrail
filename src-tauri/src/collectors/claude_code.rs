use crate::collectors::{clean_model, parse_ts_ms, read_tail, sorted_glob};
use crate::models::{Source, UsageEvent};
use crate::store::Store;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;

/// Claude Code writes one JSONL per session under ~/.claude/projects/…,
/// with the Anthropic usage object on every assistant message.
pub fn collect(store: &Store, home: &Path) -> Result<usize, String> {
    let root = home.join(".claude/projects");
    if !root.exists() {
        return Ok(0);
    }
    let files = sorted_glob(&format!("{}/**/*.jsonl", root.display()))?;
    let mut processed = 0usize;
    for path in files {
        let key = path.display().to_string();
        let offset = store.get_offset("claude_code", &key).unwrap_or(0);
        let (tail, new_offset) = read_tail(&path, offset).map_err(|e| format!("read {key}: {e}"))?;
        if tail.is_empty() {
            if new_offset != offset {
                store.set_offset("claude_code", &key, new_offset);
            }
            continue;
        }
        let is_sub_path = key.contains("subagents");
        let mut events = Vec::new();
        for line in tail.lines() {
            if !line.contains("\"usage\":{") {
                continue;
            }
            if let Some(ev) = parse_line(line, is_sub_path) {
                events.push(ev);
            }
        }
        processed += store.insert_events(&events).map_err(|e| format!("claude insert: {e}"))?;
        store.set_offset("claude_code", &key, new_offset);
    }
    Ok(processed)
}

fn parse_line(line: &str, is_sub_path: bool) -> Option<UsageEvent> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let msg = v.get("message")?;
    let usage = msg.get("usage")?;
    let get = |k: &str| usage.get(k).and_then(|x| x.as_i64()).unwrap_or(0);
    let (input, output, cr, cw) =
        (get("input_tokens"), get("output_tokens"), get("cache_read_input_tokens"), get("cache_creation_input_tokens"));
    if input + output + cr + cw == 0 {
        return None; // empty "usage":{} sidechain markers
    }
    let ts = v.get("timestamp").and_then(|x| x.as_str()).and_then(parse_ts_ms)?;
    let msg_id = msg.get("id").and_then(|x| x.as_str()).unwrap_or("");
    let request_id = v.get("requestId").and_then(|x| x.as_str()).unwrap_or("");
    let source_event_id = if !msg_id.is_empty() || !request_id.is_empty() {
        format!("{msg_id}:{request_id}")
    } else {
        let mut h = DefaultHasher::new();
        line.hash(&mut h);
        format!("h{:x}", h.finish())
    };
    Some(UsageEvent {
        source: Source::ClaudeCode,
        source_event_id,
        ts,
        session_id: v.get("sessionId").and_then(|x| x.as_str()).map(String::from),
        project: v.get("cwd").and_then(|x| x.as_str()).map(String::from),
        provider: Some("anthropic".into()),
        model: msg.get("model").and_then(|x| x.as_str()).map(clean_model),
        input_tokens: input,
        output_tokens: output,
        reasoning_tokens: usage
            .pointer("/output_tokens_details/thinking_tokens")
            .and_then(|x| x.as_i64()),
        cache_read_tokens: cr,
        cache_write_tokens: cw,
        duration_ms: None,
        ttft_ms: None,
        is_subagent: is_sub_path
            || v.get("isSidechain").and_then(|x| x.as_bool()).unwrap_or(false),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collectors::{fixture, test_home, test_store};

    fn seed(home: &Path) {
        let dir = home.join(".claude/projects/-Users-jack-proj");
        std::fs::create_dir_all(&dir).unwrap();
        let src = std::fs::read_to_string(fixture("claude_sample.jsonl")).unwrap();
        // first line lacks a trailing newline to exercise partial-line retry
        std::fs::write(dir.join("sess-a.jsonl"), src).unwrap();
    }

    #[test]
    fn parses_usage_and_skips_empty() {
        let home = test_home("claude");
        let store = test_store("claude");
        seed(&home);
        let n = collect(&store, &home).unwrap();
        assert_eq!(n, 1);
        let (input, output, cr, cw, reasoning, sub): (i64, i64, i64, i64, i64, i64) = store
            .conn()
            .query_row(
                "SELECT input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
                        COALESCE(reasoning_tokens,0), is_subagent
                 FROM usage_event",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )
            .unwrap();
        assert_eq!((input, output, cr, cw, reasoning, sub), (10, 100, 300, 200, 42, 0));
        let model: String = store
            .conn()
            .query_row("SELECT model FROM usage_event", [], |r| r.get(0))
            .unwrap();
        assert_eq!(model, "claude-opus-5");
        // nothing new on an immediate re-run
        assert_eq!(collect(&store, &home).unwrap(), 0);
    }
}
