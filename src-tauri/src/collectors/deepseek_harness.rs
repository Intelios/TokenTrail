//! DeepSeek Harness (DeepSeek's own coding agent) keeps one session per folder
//! under ~/.dsh/sessions, as a run of zstd frames over a JSONL event log. The
//! first line is the session header (`id`, `cwd`, `origin`); a model call is an
//! `assistant/message` carrying the provider's usage object. A subagent runs as
//! its own session file linked by `parentSession`, so its calls are counted
//! once, from its own file.
//!
//! The file is rewritten whole as it grows, so there is no byte offset to tail:
//! a per-file mtime watermark skips files that have not changed, and re-reads
//! upsert on (session id, message id). The folder name ("--Users-jack-…--") is
//! a lossy encoding, so the project comes from the header's `cwd`, never from
//! the path. The credentials file, `storages/` and the Electron profile are
//! deliberately not read.
use crate::collectors::{clean_model, mtime_ns, sorted_glob};
use crate::models::{Source, UsageEvent};
use crate::store::Store;
use ruzstd::decoding::StreamingDecoder;
use serde_json::Value;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

pub fn root(home: &Path) -> PathBuf {
    home.join(".dsh/sessions")
}

pub fn collect(store: &Store, home: &Path) -> Result<usize, String> {
    let root = root(home);
    if !root.exists() {
        return Ok(0);
    }
    let mut processed = 0;
    let mut errors = Vec::new();
    for path in sorted_glob(&format!("{}/*/*/session.v4.jsonl.zstd", root.display()))? {
        match collect_file(store, &path) {
            Ok(n) => processed += n,
            Err(e) => errors.push(e),
        }
    }
    if errors.is_empty() {
        Ok(processed)
    } else {
        Err(format!(
            "DeepSeek Harness: {} session(s) need attention. {}",
            errors.len(),
            errors[0]
        ))
    }
}

fn collect_file(store: &Store, path: &Path) -> Result<usize, String> {
    let key = path.display().to_string();
    let watermark = format!("deepseek_harness:mtime:{key}");
    let mtime = mtime_ns(path);
    if mtime > 0 && mtime <= store.get_watermark(&watermark) {
        return Ok(0);
    }
    let bytes = std::fs::read(path).map_err(|_| "Could not read session.".to_string())?;
    let (text, complete) = decode(&bytes);
    if text.is_empty() && !complete {
        return Err("Session data did not decompress.".into());
    }
    // A half-written tail line is retried on the next poll, like read_tail's.
    let complete_lines = match text.rfind('\n') {
        Some(end) => &text[..=end],
        None => "",
    };
    let folder_id = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("");
    let Session {
        id,
        project,
        is_subagent,
        calls,
    } = parse_session(complete_lines);
    let session_id = if id.is_empty() { folder_id.to_string() } else { id };
    if session_id.is_empty() {
        return Ok(0);
    }
    let events: Vec<UsageEvent> = calls
        .into_iter()
        .map(|c| UsageEvent {
            source: Source::DeepseekHarness,
            source_event_id: format!("{session_id}:{}", c.message_id),
            ts: c.ts,
            session_id: Some(session_id.clone()),
            project: project.clone(),
            provider: c.provider,
            provider_name: None,
            model: Some(clean_model(&c.model)),
            input_tokens: c.input,
            output_tokens: c.output,
            reasoning_tokens: None,
            cache_read_tokens: c.cache_read,
            // The harness reports no separate cache-write count.
            cache_write_tokens: 0,
            duration_ms: None,
            ttft_ms: None,
            is_subagent,
            // The provider reported these counts; nothing here is guessed.
            estimated: false,
            purpose: None,
            outcome: None,
            workspace: None,
            subagent_id: None,
            mode: None,
        })
        .collect();
    let n = store
        .insert_events(&events)
        .map_err(|e| format!("insert session: {e}"))?;
    if complete {
        store.set_watermark(&watermark, mtime);
    }
    Ok(n)
}

/// Decompress every frame in the file. Sessions are written as one frame per
/// appended batch, so a single `StreamingDecoder` stops after the first. A
/// truncated write decodes the frames before the break and reports `complete`
/// as false, which keeps the file due for another read.
fn decode(bytes: &[u8]) -> (String, bool) {
    let mut cursor = Cursor::new(bytes);
    let mut out: Vec<u8> = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    while cursor.position() < bytes.len() as u64 {
        let start = cursor.position();
        let mut decoder = match StreamingDecoder::new(&mut cursor) {
            Ok(d) => d,
            Err(_) => return (String::from_utf8_lossy(&out).into_owned(), false),
        };
        loop {
            match decoder.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => out.extend_from_slice(&buf[..n]),
                Err(_) => return (String::from_utf8_lossy(&out).into_owned(), false),
            }
        }
        if cursor.position() <= start {
            return (String::from_utf8_lossy(&out).into_owned(), false);
        }
    }
    (String::from_utf8_lossy(&out).into_owned(), true)
}

struct Call {
    ts: i64,
    message_id: String,
    provider: Option<String>,
    model: String,
    input: i64,
    output: i64,
    cache_read: i64,
}

struct Session {
    id: String,
    project: Option<String>,
    is_subagent: bool,
    calls: Vec<Call>,
}

/// Read what the log says about itself and every model call it recorded. Unknown
/// event types, and assistant messages with no measured usage, are skipped.
fn parse_session(text: &str) -> Session {
    let mut session = Session {
        id: String::new(),
        project: None,
        is_subagent: false,
        calls: Vec::new(),
    };
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match v.get("type").and_then(Value::as_str).unwrap_or("") {
            "session" => {
                session.id = v.get("id").and_then(Value::as_str).unwrap_or("").into();
                session.project = v
                    .get("cwd")
                    .and_then(Value::as_str)
                    .filter(|cwd| !cwd.is_empty())
                    .map(String::from);
                // A subagent session says so either way; keep both signals.
                session.is_subagent = v.get("origin").and_then(Value::as_str) == Some("subagent")
                    || v.get("delegationDepth").and_then(Value::as_i64).unwrap_or(0) > 0;
            }
            "assistant/message" => {
                if let Some(call) = parse_call(&v) {
                    session.calls.push(call);
                }
            }
            _ => {}
        }
    }
    session
}

fn parse_call(v: &Value) -> Option<Call> {
    let data = v.get("data")?;
    let usage = data.get("usage").filter(|u| u.is_object())?;
    let message = data.get("message")?;
    let ts = v.get("time").and_then(Value::as_i64)?;
    let message_id = message.get("id").and_then(Value::as_str).unwrap_or("");
    let model = message
        .pointer("/source/model")
        .and_then(Value::as_str)
        .unwrap_or("");
    let count = |key: &str| usage.get(key).and_then(Value::as_i64).unwrap_or(0);
    let (input, output, cache_read) = (
        count("inputTokens"),
        count("outputTokens"),
        count("cacheReadTokens"),
    );
    if ts <= 0
        || message_id.is_empty()
        || model.is_empty()
        || [input, output, cache_read].iter().any(|n| *n < 0)
        || input + output + cache_read == 0
    {
        return None;
    }
    Some(Call {
        ts,
        message_id: message_id.into(),
        provider: message
            .pointer("/source/provider")
            .and_then(Value::as_str)
            .map(String::from),
        model: model.into(),
        input,
        output,
        cache_read,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collectors::{test_home, test_store};
    use ruzstd::encoding::{compress_to_vec, CompressionLevel};
    use serde_json::json;
    use std::fs::FileTimes;
    use std::time::{Duration, SystemTime};

    fn frame(line: &Value) -> Vec<u8> {
        compress_to_vec(format!("{line}\n").as_bytes(), CompressionLevel::Fastest)
    }

    fn header(id: &str, cwd: &str, subagent: bool) -> Value {
        if subagent {
            json!({"type":"session","version":4,"id":id,"createdAt":1,"cwd":cwd,
                   "parentSession":"session-main","isSeeded":false,"origin":"subagent",
                   "delegationDepth":1,"agentPreset":"standard"})
        } else {
            json!({"type":"session","version":4,"id":id,"createdAt":1,"cwd":cwd,
                   "isSeeded":false,"delegationDepth":0,"agentPreset":"standard"})
        }
    }

    fn call(id: &str, ts: i64, input: i64, output: i64, cache_read: i64) -> Value {
        json!({"type":"assistant/message","seq":7,"time":ts,"data":{
            "turn":1,"step":1,
            "message":{"role":"assistant","content":[{"type":"text","text":"ok"}],
                       "source":{"kind":"model","provider":"ollama",
                                 "model":"deepseek-v4.1-flash:1017"},
                       "id":id},
            "usage":{"inputTokens":input,"outputTokens":output,
                     "totalTokens":input+output+cache_read,"cacheReadTokens":cache_read}
        }})
    }

    /// One frame per line, the way the app appends.
    fn session_bytes(header: &Value, lines: &[Value]) -> Vec<u8> {
        let mut bytes = frame(header);
        for line in lines {
            bytes.extend(frame(line));
        }
        bytes
    }

    fn write(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn session_path(home: &Path, folder: &str, id: &str) -> PathBuf {
        home.join(format!(".dsh/sessions/{folder}/{id}/session.v4.jsonl.zstd"))
    }

    /// A main session with two calls and a subagent session with one.
    fn seed(home: &Path) -> (PathBuf, PathBuf) {
        let main = session_bytes(
            &header("session-main", "/Users/jack/proj", false),
            &[
                json!({"type":"user/message","seq":1,"time":10,"data":{"role":"user","content":[]}}),
                call("msg-1", 1_790_000_000_000, 100, 20, 300),
                json!({"type":"tool/call","seq":3,"time":11,"data":{"name":"read"}}),
                json!({"type":"future/thing","seq":4,"time":12,"data":{"whatever":true}}),
                call("msg-2", 1_790_000_060_000, 10, 5, 0),
            ],
        );
        let sub = session_bytes(
            &header("sub-1", "/Users/jack/proj", true),
            &[call("msg-1", 1_790_000_030_000, 7, 3, 40)],
        );
        let main_path = session_path(home, "--Users-jack-proj--", "session-main");
        let sub_path = session_path(home, "--Users-jack-proj--", "sub-1");
        write(&main_path, &main);
        write(&sub_path, &sub);
        (main_path, sub_path)
    }

    fn rows(store: &Store) -> (i64, i64, i64, i64) {
        store
            .read_conn()
            .query_row(
                "SELECT COUNT(*), SUM(input_tokens), SUM(output_tokens), SUM(cache_read_tokens) FROM usage_event",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap()
    }

    #[test]
    fn reads_measured_usage_from_main_and_subagent_sessions() {
        let home = test_home("deepseek");
        let store = test_store("deepseek");
        seed(&home);
        assert_eq!(collect(&store, &home).unwrap(), 3);
        assert_eq!(rows(&store), (3, 117, 28, 340));
        let (model, provider, estimated, project): (String, String, i64, String) = store
            .read_conn()
            .query_row(
                "SELECT model, provider, estimated, project FROM usage_event
                 WHERE session_id='session-main' AND source_event_id='session-main:msg-1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            (model.as_str(), provider.as_str(), estimated, project.as_str()),
            ("deepseek-v4.1-flash:1017", "ollama", 0, "/Users/jack/proj")
        );
        // the subagent's own file flags its calls, and its twin never appears twice
        let sub: (i64, Option<String>) = store
            .read_conn()
            .query_row(
                "SELECT is_subagent, subagent_id FROM usage_event WHERE session_id='sub-1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(sub, (1, None));
    }

    #[test]
    fn unchanged_files_are_skipped_and_rereads_upsert() {
        let home = test_home("deepseek-idempotent");
        let store = test_store("deepseek-idempotent");
        let (main, _) = seed(&home);
        assert_eq!(collect(&store, &home).unwrap(), 3);
        assert_eq!(collect(&store, &home).unwrap(), 0);

        // a forced re-read of the same bytes must not duplicate a call
        store.clear_watermarks("deepseek_harness:%");
        collect(&store, &home).unwrap();
        assert_eq!(rows(&store), (3, 117, 28, 340));

        // a rewritten file with new calls is picked up again
        let grown = session_bytes(
            &header("session-main", "/Users/jack/proj", false),
            &[
                call("msg-1", 1_790_000_000_000, 100, 20, 300),
                call("msg-2", 1_790_000_060_000, 10, 5, 0),
                call("msg-3", 1_790_000_120_000, 1, 1, 0),
            ],
        );
        write(&main, &grown);
        let later = SystemTime::now() + Duration::from_secs(60);
        std::fs::OpenOptions::new()
            .write(true)
            .open(&main)
            .unwrap()
            .set_times(FileTimes::new().set_modified(later))
            .unwrap();
        assert_eq!(collect(&store, &home).unwrap(), 3);
        assert_eq!(rows(&store).0, 4);
    }

    #[test]
    fn a_truncated_write_keeps_complete_lines_and_is_retried() {
        let home = test_home("deepseek-truncated");
        let store = test_store("deepseek-truncated");
        let mut prefix = session_bytes(
            &header("session-main", "/Users/jack/proj", false),
            &[
                call("msg-1", 1_790_000_000_000, 100, 20, 300),
                call("msg-2", 1_790_000_060_000, 10, 5, 0),
            ],
        );
        // A long reply leaves the last frame big enough to cut in half.
        let mut last = call("msg-3", 1_790_000_120_000, 1, 1, 0);
        last["data"]["message"]["content"] = json!([{"type": "text", "text": "x".repeat(4000)}]);
        let last_frame = frame(&last);
        let frame_start = prefix.len();
        prefix.extend(&last_frame);
        let main = session_path(&home, "--Users-jack-proj--", "session-main");
        write(&main, &prefix[..frame_start + last_frame.len() / 2]);

        assert_eq!(collect(&store, &home).unwrap(), 2);
        assert_eq!(rows(&store).0, 2);

        // The unfinished frame was never marked read, so the next sync picks it up.
        write(&main, &prefix);
        assert_eq!(collect(&store, &home).unwrap(), 3);
        assert_eq!(rows(&store).0, 3);
    }

    #[test]
    fn messages_without_usage_are_not_generations() {
        let text = [
            header("session-x", "/tmp/p", false).to_string(),
            json!({"type":"assistant/message","time":5,"data":{"message":{"id":"m1",
                   "source":{"provider":"ollama","model":"deepseek-chat"}},"usage":{}}})
            .to_string(),
            json!({"type":"assistant/message","time":6,"data":{"usage":{"inputTokens":1,
                   "outputTokens":1},"message":{"id":"","source":{"model":"deepseek-chat"}}}})
            .to_string(),
            call("m3", 7, 0, 0, 0).to_string(),
            json!({"type":"assistant/message","time":8,"data":{"usage":{"inputTokens":5,
                   "outputTokens":6},"message":{"source":{"model":"deepseek-chat"}}}})
            .to_string(),
        ]
        .join("\n");
        let session = parse_session(&format!("{text}\n"));
        assert!(session.calls.is_empty());
        assert_eq!(session.project.as_deref(), Some("/tmp/p"));
    }

    #[test]
    fn data_that_does_not_decompress_is_reported() {
        let home = test_home("deepseek-broken");
        let store = test_store("deepseek-broken");
        let path = session_path(&home, "--tmp--", "session-broken");
        write(&path, b"this is not zstd");
        let err = collect(&store, &home).unwrap_err();
        assert!(err.starts_with("DeepSeek Harness: 1 session"), "{err}");
    }

    #[test]
    fn a_machine_without_the_app_is_a_no_op() {
        let home = test_home("deepseek-missing");
        let store = test_store("deepseek-missing");
        assert_eq!(collect(&store, &home).unwrap(), 0);
    }
}
