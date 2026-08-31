use crate::models::{Source, UsageEvent};
use crate::store::{open_readonly, Store};
use std::path::Path;

/// OpenCode (SST) stores per-message data as a JSON blob in ~/.local/share/
/// opencode/opencode.db (possibly per-channel dbs). We poll message rows;
/// session-level aggregate columns are deliberately ignored to avoid double
/// counting once messages exist.
const OVERLAP_MS: i64 = 5 * 60 * 1000;

pub fn collect(store: &Store, home: &Path) -> Result<usize, String> {
    let dir = home.join(".local/share/opencode");
    if !dir.exists() {
        return Ok(0);
    }
    let dbs = crate::collectors::sorted_glob(&format!("{}/*.db", dir.display()))?;
    let mut processed = 0usize;
    for db in dbs {
        let name = db.file_name().and_then(|n| n.to_str()).unwrap_or("db").to_string();
        let Ok(conn) = open_readonly(&db) else { continue };
        let has_messages: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='message'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n > 0)
            .unwrap_or(false);
        if !has_messages {
            continue;
        }
        let key = format!("opencode:{name}");
        let watermark = store.get_watermark(&key).saturating_sub(OVERLAP_MS);
        let mut stmt = conn
            .prepare(
                "SELECT m.id, m.session_id, m.time_created, m.data, s.directory, s.model
                 FROM message m LEFT JOIN session s ON s.id = m.session_id
                 WHERE (CASE WHEN m.time_created < 100000000000 THEN m.time_created * 1000 ELSE m.time_created END) > ?1
                 ORDER BY (CASE WHEN m.time_created < 100000000000 THEN m.time_created * 1000 ELSE m.time_created END)",
            )
            .map_err(|e| format!("opencode prepare: {e}"))?;
        let rows = stmt
            .query_map([watermark], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                ))
            })
            .map_err(|e| format!("opencode query: {e}"))?;

        let mut events = Vec::new();
        let mut max_ts = 0i64;
        for row in rows {
            let (id, session_id, time_created, data, directory, session_model) =
                row.map_err(|e| format!("opencode row: {e}"))?;
            let Some(ev) = from_message(&id, &session_id, time_created, &data, directory, session_model) else {
                continue;
            };
            max_ts = max_ts.max(ev.ts);
            events.push(ev);
        }
        processed += store.insert_events(&events).map_err(|e| format!("opencode insert: {e}"))?;
        if max_ts > 0 {
            store.set_watermark(&key, max_ts);
        }
    }
    Ok(processed)
}

fn from_message(
    id: &str,
    session_id: &Option<String>,
    time_created: i64,
    data: &str,
    directory: Option<String>,
    session_model: Option<String>,
) -> Option<UsageEvent> {
    let v: serde_json::Value = serde_json::from_str(data).ok()?;
    let tokens = v.get("tokens")?;
    let get = |keys: &[&str]| {
        for k in keys {
            if let Some(x) = tokens.get(*k).and_then(|x| x.as_i64()) {
                return x;
            }
        }
        0
    };
    let input = get(&["input", "prompt"]);
    let output = get(&["output", "completion"]);
    let cache_read = tokens.pointer("/cache/read").or_else(|| tokens.get("cache_read")).and_then(|x| x.as_i64()).unwrap_or(0);
    let cache_write = tokens.pointer("/cache/write").or_else(|| tokens.get("cache_write")).and_then(|x| x.as_i64()).unwrap_or(0);
    if input + output + cache_read + cache_write == 0 {
        return None;
    }
    // some builds store seconds instead of milliseconds
    let ts = if time_created > 0 && time_created < 100_000_000_000 { time_created * 1000 } else { time_created };
    if ts <= 0 {
        return None;
    }
    let model = ["modelID", "model_id", "model"]
        .iter()
        .find_map(|k| v.get(*k).and_then(|x| x.as_str()).map(String::from))
        .or(session_model);
    Some(UsageEvent {
        source: Source::Opencode,
        source_event_id: id.to_string(),
        ts,
        session_id: session_id.clone(),
        project: directory,
        provider: v.get("providerID").and_then(|x| x.as_str()).map(String::from),
        model,
        input_tokens: input,
        output_tokens: output,
        reasoning_tokens: tokens.get("reasoning").and_then(|x| x.as_i64()),
        cache_read_tokens: cache_read,
        cache_write_tokens: cache_write,
        duration_ms: None,
        ttft_ms: None,
        is_subagent: false,
        estimated: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collectors::{test_home, test_store};
    use rusqlite::Connection;

    fn seed(home: &Path) {
        let db = home.join(".local/share/opencode/opencode.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let c = Connection::open(&db).unwrap();
        c.execute_batch(
            "CREATE TABLE session (id TEXT PRIMARY KEY, directory TEXT, model TEXT);
             CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT NOT NULL,
               time_created INTEGER NOT NULL, data TEXT NOT NULL);
             INSERT INTO session VALUES ('s1', '/Users/jack/oc', 'anthropic/claude-sonnet-4.5');
             INSERT INTO message VALUES ('msg_1', 's1', 1754100000,
               '{\"tokens\":{\"input\":100,\"output\":50,\"cache\":{\"read\":10,\"write\":5}},\"modelID\":\"anthropic/claude-sonnet-4.5\",\"providerID\":\"anthropic\"}');",
        )
        .unwrap();
    }

    #[test]
    fn ingests_message_json() {
        let home = test_home("opencode");
        let store = test_store("opencode");
        seed(&home);
        assert_eq!(collect(&store, &home).unwrap(), 1);
        let (input, cr, cw, ts, model): (i64, i64, i64, i64, String) = store
            .conn()
            .query_row(
                "SELECT input_tokens, cache_read_tokens, cache_write_tokens, ts, model FROM usage_event",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get::<_, Option<String>>(4)?.unwrap_or_default())),
            )
            .unwrap();
        assert_eq!((input, cr, cw), (100, 10, 5));
        assert_eq!(ts, 1_754_100_000_000); // seconds were normalized to ms
        assert!(model.contains("claude"));
    }
}
