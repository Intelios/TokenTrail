use crate::collectors::{clean_model, parse_ts_ms, read_tail, sorted_glob};
use crate::models::{Source, UsageEvent};
use crate::store::Store;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

struct SessionMeta {
    session_id: Option<String>,
    cwd: Option<String>,
    is_subagent: bool,
}

/// Codex CLI rollouts: ~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl (+ flat
/// archived_sessions/). Each turn emits a token_count event; we take the
/// per-turn `last_token_usage` delta, never the cumulative total.
/// Forked threads replay the parent's events, so the dedup key includes the
/// token tuple itself: an identical (thread, ts, usage) replay is ignored.
pub fn collect(store: &Store, home: &Path) -> Result<usize, String> {
    let mut files: Vec<PathBuf> = sorted_glob(&format!("{}/.codex/sessions/**/*.jsonl", home.display()))?;
    files.extend(sorted_glob(&format!("{}/.codex/archived_sessions/*.jsonl", home.display()))?);
    if files.is_empty() {
        return Ok(0);
    }
    let mut processed = 0usize;
    for path in files {
        let key = path.display().to_string();
        let offset = store.get_offset("codex", &key).unwrap_or(0);
        let (tail, new_offset) = read_tail(&path, offset).map_err(|e| format!("read {key}: {e}"))?;
        if tail.is_empty() {
            if new_offset != offset {
                store.set_offset("codex", &key, new_offset);
            }
            continue;
        }
        let meta = read_meta(&path);
        let mut current_model: Option<String> = None;
        let mut events = Vec::new();
        for line in tail.lines() {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
            let payload = v.get("payload");
            if let Some(m) = payload.and_then(|p| p.get("model")).and_then(|x| x.as_str()) {
                current_model = Some(clean_model(m));
            }
            if v.get("type").and_then(|x| x.as_str()) != Some("event_msg") {
                continue;
            }
            if payload.and_then(|p| p.get("type")).and_then(|x| x.as_str()) != Some("token_count") {
                continue;
            }
            let Some(info) = payload.and_then(|p| p.get("info")) else { continue };
            let Some((input, cr, cw, out, reasoning)) =
                info.get("last_token_usage").and_then(usage_from)
            else {
                continue;
            };
            let Some(ts) = v.get("timestamp").and_then(|x| x.as_str()).and_then(parse_ts_ms) else {
                continue;
            };
            let sid = meta.session_id.clone().unwrap_or_default();
            let model = current_model
                .clone()
                .or_else(|| store.latest_session_model("codex", &sid));
            events.push(UsageEvent {
                source: Source::Codex,
                source_event_id: format!("{sid}:{ts}:{input}:{out}"),
                ts,
                session_id: (!sid.is_empty()).then(|| sid),
                project: meta.cwd.clone(),
                provider: Some("openai".into()),
                model,
                input_tokens: input,
                output_tokens: out,
                reasoning_tokens: Some(reasoning),
                cache_read_tokens: cr,
                cache_write_tokens: cw,
                duration_ms: None,
                ttft_ms: None,
                is_subagent: meta.is_subagent,
            });
        }
        processed += store.insert_events(&events).map_err(|e| format!("codex insert: {e}"))?;
        store.set_offset("codex", &key, new_offset);
    }
    Ok(processed)
}

fn usage_from(v: &serde_json::Value) -> Option<(i64, i64, i64, i64, i64)> {
    let get = |k: &str| v.get(k).and_then(|x| x.as_i64()).unwrap_or(0);
    let (input, cr, cw, out, reasoning) = (
        get("input_tokens"),
        get("cached_input_tokens"),
        get("cache_write_input_tokens"),
        get("output_tokens"),
        get("reasoning_output_tokens"),
    );
    if input + cr + cw + out == 0 {
        return None;
    }
    Some((input, cr, cw, out, reasoning))
}

fn read_meta(path: &Path) -> SessionMeta {
    let mut meta = SessionMeta {
        session_id: filename_thread_id(path),
        cwd: None,
        is_subagent: false,
    };
    let Ok(file) = std::fs::File::open(path) else { return meta };
    let mut first = String::new();
    if BufReader::new(file).read_line(&mut first).is_err() {
        return meta;
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&first) else { return meta };
    let Some(p) = v.get("payload") else { return meta };
    if let Some(sid) = p.get("session_id").and_then(|x| x.as_str()).or_else(|| p.get("id").and_then(|x| x.as_str())) {
        meta.session_id = Some(sid.to_string());
    }
    meta.cwd = p.get("cwd").and_then(|x| x.as_str()).map(String::from);
    meta.is_subagent = p.get("thread_source").and_then(|x| x.as_str()) == Some("subagent");
    meta
}

/// rollout-2026-08-16T18-38-48-01a00ba7-9af3-7c41-aeee-68a790e1b8a5.jsonl
/// -> 01a00ba7-9af3-7c41-aeee-68a790e1b8a5 (a uuid v7 is the last 5 groups)
fn filename_thread_id(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let parts: Vec<&str> = stem.split('-').collect();
    if parts.len() >= 5 {
        Some(parts[parts.len() - 5..].join("-"))
    } else {
        Some(stem.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collectors::{fixture, test_home, test_store};

    fn seed(home: &Path) {
        let dir = home.join(".codex/sessions/2026/08/02");
        std::fs::create_dir_all(&dir).unwrap();
        let src = std::fs::read_to_string(fixture("codex_sample.jsonl")).unwrap();
        std::fs::write(
            dir.join("rollout-2026-08-02T12-00-00-11111111-2222-3333-4444-555555555555.jsonl"),
            src,
        )
        .unwrap();
    }

    #[test]
    fn takes_turn_deltas_and_model() {
        let home = test_home("codex");
        let store = test_store("codex");
        seed(&home);
        assert_eq!(collect(&store, &home).unwrap(), 1);
        let (input, cr, cw, out, reasoning, sub): (i64, i64, i64, i64, i64, i64) = store
            .conn()
            .query_row(
                "SELECT input_tokens, cache_read_tokens, cache_write_tokens, output_tokens,
                        COALESCE(reasoning_tokens,0), is_subagent
                 FROM usage_event",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )
            .unwrap();
        assert_eq!((input, cr, cw, out, reasoning, sub), (50, 10, 5, 20, 7, 0));
        let (model, project): (String, String) = store
            .conn()
            .query_row("SELECT model, project FROM usage_event", [], |r| {
                Ok((r.get::<_, Option<String>>(0)?.unwrap_or_default(), r.get::<_, Option<String>>(1)?.unwrap_or_default()))
            })
            .unwrap();
        assert_eq!(model, "gpt-5.6-luna");
        assert_eq!(project, "/Users/jack/cx");
        assert_eq!(collect(&store, &home).unwrap(), 0);
    }
}
