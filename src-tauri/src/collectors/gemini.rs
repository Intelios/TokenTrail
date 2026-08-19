use crate::collectors::{parse_ts_ms, read_tail, sorted_glob};
use crate::models::{Source, UsageEvent};
use crate::store::Store;
use serde_json::Value;
use std::path::Path;

/// Gemini CLI persists chats under ~/.gemini/tmp/<hash>/chats/ as JSONL
/// (current) or whole-file JSON (legacy). Per-message token counts live in
/// the "tokens" object. Many installs never write these files; the collector
/// simply stays dormant until they appear.
pub fn collect(store: &Store, home: &Path) -> Result<usize, String> {
    let root = home.join(".gemini/tmp");
    if !root.exists() {
        return Ok(0);
    }
    // chats live one project-hash level down: tmp/<hash>/chats/session-*.{jsonl,json}
    const DEPTH: &str = "**";
    let mut files = sorted_glob(&format!("{}/{DEPTH}/chats/*.jsonl", root.display()))?;
    files.extend(sorted_glob(&format!("{}/{DEPTH}/chats/*.json", root.display()))?);
    let mut processed = 0usize;
    for path in files {
        let key = path.display().to_string();
        let is_json = key.ends_with(".json");
        let offset = store.get_offset("gemini", &key).unwrap_or(0);
        let (tail, new_offset) = read_tail(&path, offset).map_err(|e| format!("read {key}: {e}"))?;
        if tail.is_empty() {
            if new_offset != offset {
                store.set_offset("gemini", &key, new_offset);
            }
            continue;
        }
        let mut events = Vec::new();
        if is_json {
            let Ok(v) = serde_json::from_str::<Value>(&tail) else {
                // incomplete or unparsable file: retry next poll, don't advance
                continue;
            };
            let record_ts = v.get("startTime").and_then(|x| x.as_str()).and_then(parse_ts_ms);
            if let Some(messages) = v.get("messages").and_then(|m| m.as_array()) {
                for msg in messages {
                    if let Some(ev) = parse_message(msg, record_ts) {
                        events.push(ev);
                    }
                }
            }
        } else {
            for line in tail.lines() {
                if !line.contains("\"tokens\"") {
                    continue;
                }
                let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
                let ts = v.get("timestamp").and_then(|x| x.as_str()).and_then(parse_ts_ms);
                if let Some(ev) = parse_message(&v, ts) {
                    events.push(ev);
                }
            }
        }
        processed += store.insert_events(&events).map_err(|e| format!("gemini insert: {e}"))?;
        store.set_offset("gemini", &key, new_offset);
    }
    Ok(processed)
}

fn parse_message(v: &Value, fallback_ts: Option<i64>) -> Option<UsageEvent> {
    let tokens = v.get("tokens")?;
    let get = |keys: &[&str]| {
        for k in keys {
            if let Some(x) = tokens.get(*k).and_then(|x| x.as_i64()) {
                return x;
            }
        }
        0
    };
    let input = get(&["input", "promptTokenCount"]);
    let output = get(&["output", "candidatesTokenCount"]);
    let cache_read = get(&["cached", "cachedContentTokenCount"]);
    let thoughts = get(&["thoughts", "thoughtsTokenCount"]);
    if input + output + cache_read + thoughts == 0 {
        return None;
    }
    let ts = v
        .get("timestamp")
        .and_then(|x| x.as_str())
        .and_then(parse_ts_ms)
        .or(fallback_ts)?;
    let session = v.get("sessionId").and_then(|x| x.as_str()).unwrap_or("");
    Some(UsageEvent {
        source: Source::Gemini,
        source_event_id: format!("{session}:{ts}:{input}:{output}"),
        ts,
        session_id: (!session.is_empty()).then(|| session.to_string()),
        project: None,
        provider: Some("google".into()),
        model: v.get("model").and_then(|x| x.as_str()).map(String::from),
        input_tokens: input,
        output_tokens: output,
        reasoning_tokens: Some(thoughts).filter(|t| *t > 0),
        cache_read_tokens: cache_read,
        cache_write_tokens: 0,
        duration_ms: None,
        ttft_ms: None,
        is_subagent: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collectors::{fixture, test_home, test_store};

    fn seed(home: &Path) {
        let dir = home.join(".gemini/tmp/abc123/chats");
        std::fs::create_dir_all(&dir).unwrap();
        let src = std::fs::read_to_string(fixture("gemini_sample.jsonl")).unwrap();
        std::fs::write(dir.join("session-1.jsonl"), src).unwrap();
    }

    #[test]
    fn parses_jsonl_tokens() {
        let home = test_home("gemini");
        let store = test_store("gemini");
        seed(&home);
        assert_eq!(collect(&store, &home).unwrap(), 1);
        let (input, output, cr): (i64, i64, i64) = store
            .conn()
            .query_row(
                "SELECT input_tokens, output_tokens, cache_read_tokens FROM usage_event",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!((input, output, cr), (100, 50, 25));
    }
}
