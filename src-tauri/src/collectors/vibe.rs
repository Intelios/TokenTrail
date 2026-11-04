//! Mistral Vibe 3's unified Harness store. A provider reply appears both as an
//! action_result and a replayable core_input: only the latter is counted, using
//! (session_id, action_id). Projection totals and transcript chunks are not calls.
//!
//! Vibe retains only two journal segments. Calls already pruned before TokenTrail
//! sees them cannot be reconstructed with honest per-request dates/models.
use crate::collectors::{clean_model, mtime_ns, read_tail, sorted_glob};
use crate::models::{Source, UsageEvent};
use crate::store::Store;
use serde_json::Value;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

pub fn root(home: &Path) -> PathBuf {
    home.join(".vibe/logs/session")
}

pub fn collect(store: &Store, home: &Path) -> Result<usize, String> {
    let root = root(home);
    if !root.exists() {
        return Ok(0);
    }
    let mut processed = 0;
    let mut errors = Vec::new();
    // Deliberately avoid recursive discovery: plugin caches and exports can
    // contain copies of sessions. Subagents have their own unified session id.
    for meta in sorted_glob(&format!("{}/unified/*/meta.json", root.display()))? {
        match collect_session(store, &meta) {
            Ok(n) => processed += n,
            Err(e) => errors.push(e),
        }
    }
    if errors.is_empty() {
        Ok(processed)
    } else {
        Err(format!(
            "Vibe: {} session(s) need attention. {}",
            errors.len(),
            errors[0]
        ))
    }
}

fn read_json(path: &Path) -> Result<Value, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("parse {}: {e}", path.display()))
}

fn collect_session(store: &Store, meta_path: &Path) -> Result<usize, String> {
    let dir = meta_path.parent().ok_or("Vibe metadata has no parent")?;
    let journals = sorted_glob(&format!("{}/journal/*.jsonl", dir.display()))?;
    let snapshots = sorted_glob(&format!(
        "{}/generations/*/runtime-state.json",
        dir.display()
    ))?;
    let key = format!("vibe:session:{}", dir.display());
    // Re-read a changed session as a unit: time, model and cwd changes can
    // precede a completion in another segment. No cursor can strand that context.
    let mut hasher = DefaultHasher::new();
    for path in
        std::iter::once(meta_path).chain(journals.iter().chain(&snapshots).map(PathBuf::as_path))
    {
        path.hash(&mut hasher);
        mtime_ns(path).hash(&mut hasher);
        std::fs::metadata(path)
            .map_err(|e| format!("stat {}: {e}", path.display()))?
            .len()
            .hash(&mut hasher);
    }
    let fingerprint = hasher.finish() as i64;
    if store.get_watermark(&key) == fingerprint {
        return Ok(0);
    }
    let meta = read_json(meta_path)?;
    let session_id = string(&meta, "/session_id").ok_or("Vibe metadata lacks session_id")?;
    let mut context = Context {
        project: string(&meta, "/environment/working_directory"),
        model: string(&meta, "/config/model_name")
            .or_else(|| string(&meta, "/config/active_model")),
        subagent: string(&meta, "/parent_session_id").is_some(),
    };
    // Snapshots provide the cwd/model baseline for each retained segment.
    // Metadata changes within a segment are journalled as session_relocated.
    let mut baselines = Vec::new();
    for path in snapshots {
        let snapshot = read_json(&path)?;
        if let (Some(seq), Some(metadata)) = (
            snapshot["snapshot_sequence"].as_i64(),
            snapshot.get("session_metadata"),
        ) {
            baselines.push((seq, metadata.clone()));
        }
    }
    baselines.sort_by_key(|(seq, _)| *seq);
    if let Some((_, baseline)) = baselines.first() {
        context.update(baseline, &meta);
    }
    let mut baseline_index = 0;
    let mut events = Vec::new();
    let mut complete = true;
    for path in journals {
        let (text, offset) =
            read_tail(&path, 0).map_err(|e| format!("read {}: {e}", path.display()))?;
        complete &= offset
            == std::fs::metadata(&path)
                .map_err(|e| format!("stat {}: {e}", path.display()))?
                .len();
        for line in text.lines() {
            let Ok(record) = serde_json::from_str::<Value>(line) else {
                // Retry malformed records too, rather than permanently caching
                // a read that may have raced a repair/publication.
                complete = false;
                continue;
            };
            let Some(sequence) = record["sequence"].as_i64() else {
                continue;
            };
            while let Some((seq, baseline)) = baselines.get(baseline_index) {
                if *seq >= sequence {
                    break;
                }
                context.update(baseline, &meta);
                baseline_index += 1;
            }
            if record["type"] == "session_relocated" {
                if let Some(metadata) = record.pointer("/payload/metadata") {
                    context.update(metadata, &meta);
                }
            } else if let Some(event) = parse_record(&record, &session_id, &context) {
                events.push(event);
            }
        }
    }
    let n = store
        .insert_events(&events)
        .map_err(|e| format!("Vibe insert: {e}"))?;
    if complete {
        store.set_watermark(&key, fingerprint);
    }
    Ok(n)
}

fn string(value: &Value, pointer: &str) -> Option<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

struct Context {
    project: Option<String>,
    model: Option<String>,
    subagent: bool,
}

impl Context {
    fn update(&mut self, metadata: &Value, sidecar: &Value) {
        self.project = string(metadata, "/cwd");
        self.subagent = string(metadata, "/parent_session_id").is_some();
        // A pin is an alias. Resolve the sidecar's matching alias to its API
        // name; keep older pins as recorded, never relabel them as today's model.
        let pin = string(metadata, "/active_model");
        self.model = if pin.is_none() || pin == string(sidecar, "/config/active_model") {
            string(sidecar, "/config/model_name")
                .or_else(|| string(sidecar, "/config/active_model"))
                .or(pin)
        } else {
            pin
        };
    }
}

fn parse_record(record: &Value, session_id: &str, context: &Context) -> Option<UsageEvent> {
    if record["type"] != "core_input" || record["recovery_journal_record_version"] != 1 {
        return None;
    }
    let input = record.pointer("/payload/input")?;
    let command = input.get("command")?;
    if command["type"] != "completion_succeeded" {
        return None;
    }
    let action_id = string(command, "/action_id")?;
    let ts = input
        .pointer("/determinism/time_unix_ms")?
        .as_i64()
        .filter(|ts| *ts > 0)?;
    let usage = command.pointer("/result/usage")?;
    let count = |key: &str| -> Option<i64> {
        match usage.get(key) {
            None => Some(0),
            Some(n) => n.as_i64().filter(|n| *n >= 0),
        }
    };
    let input_tokens = usage.get("input_tokens")?.as_i64().filter(|n| *n >= 0)?;
    let output_tokens = usage.get("output_tokens")?.as_i64().filter(|n| *n >= 0)?;
    let cache_read_tokens = count("cached_input_tokens")?;
    let cache_write_tokens = count("cache_write_tokens")?;
    let reasoning_tokens = count("reasoning_tokens")?;
    let cached = cache_read_tokens.checked_add(cache_write_tokens)?;
    if cached > input_tokens || input_tokens.checked_add(output_tokens)? == 0 {
        return None;
    }
    // Harness input includes cache hits; output already includes reasoning.
    // Keep the four total-token buckets disjoint, as the other collectors do.
    Some(UsageEvent {
        source: Source::Vibe,
        source_event_id: format!("{session_id}:{action_id}"),
        ts,
        session_id: Some(session_id.into()),
        project: context.project.clone(),
        // Unified journals do not identify the provider route. A configured
        // Mistral model can also be served by a custom endpoint.
        provider: None,
        provider_name: None,
        model: context.model.as_deref().map(clean_model),
        input_tokens: input_tokens - cached,
        output_tokens,
        reasoning_tokens: usage.get("reasoning_tokens").map(|_| reasoning_tokens),
        cache_read_tokens,
        cache_write_tokens,
        duration_ms: None,
        ttft_ms: None,
        is_subagent: context.subagent,
        estimated: false,
        purpose: None,
        outcome: None,
        workspace: None,
        subagent_id: None,
        mode: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collectors::{fixture, test_home, test_store};
    use serde_json::json;

    fn context() -> Context {
        Context {
            project: Some("/work/app".into()),
            model: Some("mistral-large-4-0".into()),
            subagent: false,
        }
    }

    fn reply(id: &str) -> Value {
        json!({"recovery_journal_record_version": 1, "sequence": 3, "type": "core_input", "payload": {
            "input": {"determinism": {"time_unix_ms": 1793756421639_i64}, "command": {
                "type": "completion_succeeded", "action_id": id, "result": {"usage": {
                    "input_tokens": 9352, "output_tokens": 38, "cached_input_tokens": 3072,
                    "cache_write_tokens": 100, "reasoning_tokens": 22, "total_tokens": 9390
                }}
            }}
        }})
    }

    fn seed(home: &Path, session: &str, parent: Option<&str>) -> PathBuf {
        let dir = root(home).join("unified").join(session);
        std::fs::create_dir_all(dir.join("journal")).unwrap();
        std::fs::write(
            dir.join("meta.json"),
            json!({
                "session_id":session, "parent_session_id":parent,
                "environment":{"working_directory":"/work/app"},
                "config":{"active_model":"large-alias", "model_name":"mistral-large-4-0"}
            })
            .to_string(),
        )
        .unwrap();
        dir
    }

    fn write_records(path: &Path, records: &[Value]) {
        let text = records.iter().map(|r| format!("{r}\n")).collect::<String>();
        std::fs::write(path, text).unwrap();
    }

    fn count(store: &Store) -> i64 {
        store
            .read_conn()
            .query_row(
                "SELECT COUNT(*) FROM usage_event WHERE source='vibe'",
                [],
                |r| r.get(0),
            )
            .unwrap()
    }

    #[test]
    fn measured_counts_have_disjoint_cache_and_reasoning_buckets() {
        let event = parse_record(&reply("call-a"), "session-a", &context()).unwrap();
        assert_eq!(event.source_event_id, "session-a:call-a");
        assert_eq!(event.ts, 1793756421639);
        assert_eq!(event.input_tokens, 6180);
        assert_eq!(event.cache_read_tokens, 3072);
        assert_eq!(event.cache_write_tokens, 100);
        assert_eq!(event.output_tokens, 38);
        assert_eq!(event.reasoning_tokens, Some(22));
        assert_eq!(
            event.input_tokens
                + event.cache_read_tokens
                + event.cache_write_tokens
                + event.output_tokens,
            9390
        );
        assert!(!event.estimated);
        assert_eq!(event.model.as_deref(), Some("mistral-large-4-0"));
    }

    #[test]
    fn an_unpinned_startup_snapshot_uses_the_recorded_default_model() {
        let mut ctx = context();
        let sidecar =
            json!({"config":{"active_model":"large-alias", "model_name":"mistral-large-4-0"}});
        ctx.update(&json!({"cwd":"/work/app", "active_model":null}), &sidecar);
        assert_eq!(ctx.model.as_deref(), Some("mistral-large-4-0"));
        let alias_only = json!({"config":{"active_model":"my-model"}});
        ctx.update(
            &json!({"cwd":"/work/app", "active_model":null}),
            &alias_only,
        );
        assert_eq!(ctx.model.as_deref(), Some("my-model"));
    }

    #[test]
    fn rejects_missing_invalid_and_non_completion_usage() {
        for pointer in ["/payload/input/command/type", "/type"] {
            let mut record = reply("a");
            *record.pointer_mut(pointer).unwrap() = json!("action_result");
            assert!(parse_record(&record, "s", &context()).is_none());
        }
        for (key, value) in [
            ("input_tokens", json!(null)),
            ("input_tokens", json!(-1)),
            ("output_tokens", json!("38")),
            ("cached_input_tokens", json!(10000)),
            ("cache_write_tokens", json!(-1)),
            ("reasoning_tokens", json!(-1)),
            ("input_tokens", json!(i64::MAX)),
            ("output_tokens", json!(i64::MAX)),
        ] {
            let mut record = reply("a");
            record["payload"]["input"]["command"]["result"]["usage"][key] = value;
            assert!(parse_record(&record, "s", &context()).is_none(), "{key}");
        }
        let mut record = reply("");
        assert!(parse_record(&record, "s", &context()).is_none());
        record["payload"]["input"]["command"]["action_id"] = json!("a");
        record["payload"]["input"]["determinism"]["time_unix_ms"] = json!(0);
        assert!(parse_record(&record, "s", &context()).is_none());
    }

    #[test]
    fn repeated_sync_replayed_records_and_rescans_are_idempotent() {
        let home = test_home("vibe-replay");
        let store = test_store("vibe-replay-db");
        let dir = seed(&home, "session-a", None);
        std::fs::copy(fixture("vibe_sample.jsonl"), dir.join("journal/0001.jsonl")).unwrap();
        assert_eq!(collect(&store, &home).unwrap(), 1);
        assert_eq!(collect(&store, &home).unwrap(), 0);
        write_records(
            &dir.join("journal/0002.jsonl"),
            &[reply("call-a"), reply("call-b")],
        );
        collect(&store, &home).unwrap();
        assert_eq!(count(&store), 2);
        store.clear_watermarks("vibe:%");
        collect(&store, &home).unwrap();
        assert_eq!(count(&store), 2);
        let status = crate::collectors::source_status(&home)
            .into_iter()
            .find(|s| s.source == "vibe")
            .unwrap();
        assert!(status.found);
        assert_eq!(status.display, "Vibe");
        assert_eq!(Source::from_str("vibe"), Some(Source::Vibe));
        assert_eq!(serde_json::to_string(&Source::Vibe).unwrap(), "\"vibe\"");
    }

    #[test]
    fn partial_tail_is_retried_and_new_calls_survive_segment_pruning() {
        let home = test_home("vibe-tail");
        let store = test_store("vibe-tail-db");
        let dir = seed(&home, "s", None);
        let path = dir.join("journal/0001.jsonl");
        std::fs::write(&path, format!("{}\n{}", reply("a"), reply("b"))).unwrap();
        collect(&store, &home).unwrap();
        assert_eq!(count(&store), 1);
        write_records(&path, &[reply("a"), reply("b")]);
        collect(&store, &home).unwrap();
        assert_eq!(count(&store), 2);
        std::fs::remove_file(&path).unwrap();
        write_records(&dir.join("journal/0010.jsonl"), &[reply("c")]);
        collect(&store, &home).unwrap();
        assert_eq!(count(&store), 3);
    }

    #[test]
    fn subagent_calls_are_separate_and_unrelated_jsonl_is_ignored() {
        let home = test_home("vibe-subagent");
        let store = test_store("vibe-subagent-db");
        for (session, parent) in [("parent", None), ("child", Some("parent"))] {
            let dir = seed(&home, session, parent);
            write_records(&dir.join("journal/0001.jsonl"), &[reply("same-action-id")]);
        }
        let cache = root(&home).join("plugins/cache");
        std::fs::create_dir_all(&cache).unwrap();
        write_records(&cache.join("duplicate.jsonl"), &[reply("same-action-id")]);
        collect(&store, &home).unwrap();
        assert_eq!(count(&store), 2);
        let subagents: i64 = store
            .read_conn()
            .query_row("SELECT SUM(is_subagent) FROM usage_event", [], |r| r.get(0))
            .unwrap();
        assert_eq!(subagents, 1);
    }

    #[test]
    fn snapshot_baseline_and_journal_changes_keep_historical_models_and_projects() {
        let home = test_home("vibe-metadata");
        let store = test_store("vibe-metadata-db");
        let dir = seed(&home, "s", None);
        let generation = dir.join("generations/0001");
        std::fs::create_dir_all(&generation).unwrap();
        std::fs::write(generation.join("runtime-state.json"), json!({"snapshot_sequence":0,
            "session_metadata":{"cwd":"/work/old", "active_model":"old-model", "parent_session_id":null}
        }).to_string()).unwrap();
        let mut first = reply("a");
        first["sequence"] = json!(1);
        let moved = json!({"sequence":2, "type":"session_relocated", "payload":{"metadata":{
            "cwd":"/work/new", "active_model":"large-alias", "parent_session_id":null
        }}});
        write_records(&dir.join("journal/0001.jsonl"), &[first, moved, reply("b")]);
        collect(&store, &home).unwrap();
        let conn = store.read_conn();
        let old: (String, String) = conn
            .query_row(
                "SELECT model,project FROM usage_event WHERE source_event_id='s:a'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        let new: (String, String) = conn
            .query_row(
                "SELECT model,project FROM usage_event WHERE source_event_id='s:b'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(old, ("old-model".into(), "/work/old".into()));
        assert_eq!(new, ("mistral-large-4-0".into(), "/work/new".into()));
    }

    #[test]
    fn absent_app_is_a_no_op_and_broken_sessions_do_not_block_healthy_ones() {
        let home = test_home("vibe-errors");
        let store = test_store("vibe-errors-db");
        assert_eq!(collect(&store, &home).unwrap(), 0);
        let bad = seed(&home, "bad", None);
        std::fs::write(bad.join("meta.json"), "{").unwrap();
        let good = seed(&home, "good", None);
        write_records(&good.join("journal/0001.jsonl"), &[reply("a")]);
        assert!(collect(&store, &home)
            .unwrap_err()
            .starts_with("Vibe: 1 session"));
        assert_eq!(count(&store), 1);
    }

    #[test]
    #[ignore = "reads local Vibe history into an isolated temporary TokenTrail database"]
    fn local_vibe_history() {
        let home = PathBuf::from(
            std::env::var("VIBE_TEST_HOME").expect("set VIBE_TEST_HOME to the Vibe user's home"),
        );
        let store = test_store("vibe-local-smoke");
        collect(&store, &home).unwrap();
        let totals: (i64, i64, i64, i64, i64) = store.read_conn().query_row(
            "SELECT COUNT(*), COUNT(DISTINCT session_id), SUM(input_tokens + output_tokens + cache_read_tokens + cache_write_tokens), SUM(cache_read_tokens), SUM(reasoning_tokens) FROM usage_event WHERE source='vibe'",
            [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))
        ).unwrap();
        assert!(totals.0 > 0);
        let unknown_models: i64 = store
            .read_conn()
            .query_row(
                "SELECT COUNT(*) FROM usage_event WHERE source='vibe' AND model IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        println!("Vibe calls with no recorded model: {unknown_models}");
        println!("Vibe local (calls, sessions, total, cache read, reasoning): {totals:?}");
        assert_eq!(collect(&store, &home).unwrap(), 0);
    }
}
