use crate::collectors::{clean_model, parse_ts_ms};
use crate::models::{Source, UsageEvent};
use crate::store::{open_readonly, Store};
use std::collections::HashSet;
use std::path::Path;

/// Devin CLI (Cognition) keeps one SQLite store at ~/.local/share/devin/cli/
/// sessions.db. Every chat message is a message_nodes row whose chat_message
/// column is a JSON blob; assistant rows carry the generation's token counts
/// in metadata.metrics. The node forest stores each assistant message twice
/// (two nodes, one message_id), so event identity is (session_id, message_id)
/// and the twin must not be counted as a second generation. transcripts/*.json
/// are exports of the same conversations and are deliberately not read.
///
/// While the CLI runs, everything lives in sessions.db's WAL (the main file
/// can be near-empty). A readonly connection reads the WAL fine while the CLI
/// holds the database open; after a crash the WAL is unrecoverable without
/// write access, the immutable fallback sees no tables, and the sync quietly
/// reports 0 until the CLI next opens and checkpoints its database.
///
/// A row can be readable before its generation finishes, so every sync
/// re-reads an overlap window of recent rows and relies on upserts.
const OVERLAP_ROWS: i64 = 500;

pub fn collect(store: &Store, home: &Path) -> Result<usize, String> {
    let db = home.join(".local/share/devin/cli/sessions.db");
    if !db.exists() {
        return Ok(0);
    }
    let conn = open_readonly(&db).map_err(|e| format!("open devin db: {e}"))?;
    let has_messages: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='message_nodes'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n > 0)
        .unwrap_or(false);
    if !has_messages {
        return Ok(0);
    }
    // row_id is AUTOINCREMENT, so within one database it only ever grows — but
    // a wiped-and-reinstalled Devin starts over at 1, below any watermark this
    // store already holds, and would be invisible forever. The same reset
    // read_tail detects for files (length < offset): start over.
    let max_row_id: i64 = conn
        .query_row("SELECT COALESCE(MAX(row_id), 0) FROM message_nodes", [], |r| r.get(0))
        .map_err(|e| format!("devin max row: {e}"))?;
    if max_row_id < store.get_watermark("devin") {
        store.set_watermark("devin", 0);
    }
    let watermark = store.get_watermark("devin").saturating_sub(OVERLAP_ROWS);

    let mut stmt = conn
        .prepare(
            "SELECT n.row_id, n.session_id, n.chat_message, n.created_at, s.working_directory, s.model
             FROM message_nodes n LEFT JOIN sessions s ON s.id = n.session_id
             WHERE n.row_id > ?1
             ORDER BY n.row_id",
        )
        .map_err(|e| format!("devin prepare: {e}"))?;
    let rows = stmt
        .query_map([watermark], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
            ))
        })
        .map_err(|e| format!("devin query: {e}"))?;

    let mut events = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut max_row_id = 0i64;
    for row in rows {
        let (row_id, session_id, chat_message, row_created_at, cwd, session_model) =
            row.map_err(|e| format!("devin row: {e}"))?;
        max_row_id = max_row_id.max(row_id);
        let Some(ev) = from_message(&session_id, &chat_message, row_created_at, cwd, session_model)
        else {
            continue;
        };
        // The twin node shares the message_id: one generation, one event.
        if seen.insert(ev.source_event_id.clone()) {
            events.push(ev);
        }
    }
    let processed = store.insert_events(&events).map_err(|e| format!("devin insert: {e}"))?;
    if max_row_id > 0 {
        store.set_watermark("devin", max_row_id);
    }
    Ok(processed)
}

/// One assistant message -> one usage event. Only rows with an inference
/// metrics object count; anything else in the forest (system prompts, user
/// turns, tool results, interrupted generations) is skipped.
fn from_message(
    session_id: &str,
    chat_message: &str,
    row_created_at: i64,
    cwd: Option<String>,
    session_model: Option<String>,
) -> Option<UsageEvent> {
    let v: serde_json::Value = serde_json::from_str(chat_message).ok()?;
    if v.get("role").and_then(|x| x.as_str()) != Some("assistant") {
        return None;
    }
    let message_id = v.get("message_id")?.as_str()?;
    let meta = v.get("metadata")?;
    let metrics = meta.get("metrics")?;
    let get = |k: &str| metrics.get(k).and_then(|x| x.as_i64()).unwrap_or(0);
    // input_tokens excludes the cache: 683 + 12064 read back as the 12747
    // tokens the CLI reports preceding the next turn.
    let input = get("input_tokens");
    let output = get("output_tokens");
    let cache_read = get("cache_read_tokens");
    let cache_write = get("cache_creation_tokens");
    if input + output + cache_read + cache_write == 0 {
        return None;
    }
    // metadata.created_at is the generation clock; the row's own created_at is
    // the DB flush time, so it is only a last-resort approximation.
    let ts = meta
        .get("created_at")
        .and_then(|x| x.as_str())
        .and_then(parse_ts_ms)
        .unwrap_or_else(|| row_created_at.saturating_mul(1000));
    if ts <= 0 {
        return None;
    }
    let model = meta
        .get("generation_model")
        .and_then(|x| x.as_str())
        .map(clean_model)
        .or_else(|| session_model.as_deref().map(clean_model));
    // Subagent chains would be distinguishable here once Devin records them
    // in message_nodes; today subagent_heads stays empty for CLI sessions.
    Some(UsageEvent {
        source: Source::Devin,
        source_event_id: format!("{session_id}:{message_id}"),
        ts,
        session_id: Some(session_id.to_string()),
        project: cwd,
        provider: None,
        model,
        input_tokens: input,
        output_tokens: output,
        reasoning_tokens: None, // output_tokens already includes thinking
        cache_read_tokens: cache_read,
        cache_write_tokens: cache_write,
        duration_ms: metrics.get("total_time_ms").and_then(|x| x.as_i64()),
        ttft_ms: metrics.get("ttft_ms").and_then(|x| x.as_i64()),
        is_subagent: false,
        estimated: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collectors::{test_home, test_store};

    fn assistant(message_id: &str) -> String {
        format!(
            concat!(
                r#"{{"message_id":"{message_id}","role":"assistant","content":"Hello!","tool_calls":[],"#,
                r#""thinking":{{"thinking":"...","signature":""}},"#,
                r#""metadata":{{"num_tokens":95,"request_id":"d064fd16-5dff-47eb-a4cd-02f4ed6a3cc3","#,
                r#""metrics":{{"ttft_ms":1438,"total_time_ms":1885,"input_tokens":683,"output_tokens":95,"#,
                r#""cache_read_tokens":12064,"cache_creation_tokens":null}},"finish_reason":"stop","#,
                r#""created_at":"2026-09-16T12:53:14.012948Z","generation_model":"swe-1-6-slow","#,
                r#""telemetry":{{"source":"assistant","operation":"inference"}}}}}}"#
            ),
            message_id = message_id,
        )
    }

    fn seed(home: &Path) {
        let db = home.join(".local/share/devin/cli/sessions.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let c = rusqlite::Connection::open(&db).unwrap();
        c.execute_batch(
            "CREATE TABLE sessions (
               id TEXT PRIMARY KEY, working_directory TEXT NOT NULL, backend_type TEXT NOT NULL,
               model TEXT NOT NULL, agent_mode TEXT NOT NULL, created_at INTEGER NOT NULL,
               last_activity_at INTEGER NOT NULL, title TEXT, main_chain_id INTEGER,
               shell_last_seen_index INTEGER DEFAULT 0, cogs_json TEXT, workspace_dirs TEXT,
               hidden INTEGER NOT NULL DEFAULT 0, metadata TEXT);
             CREATE TABLE message_nodes (
               row_id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL,
               node_id INTEGER NOT NULL, parent_node_id INTEGER, chat_message TEXT NOT NULL,
               created_at INTEGER NOT NULL, metadata TEXT, UNIQUE(session_id, node_id));
             INSERT INTO sessions VALUES ('fossil-echium', '/Users/jack', 'windsurf',
               'swe-1-6-slow', 'normal', 1789563192, 1789563194, 'Hi', 23, 0, NULL, NULL, 0, NULL);",
        )
        .unwrap();
        let insert = |c: &rusqlite::Connection, sid: &str, node: i64, parent: Option<i64>, json: &str| {
            c.execute(
                "INSERT INTO message_nodes (session_id, node_id, parent_node_id, chat_message, created_at) VALUES (?1,?2,?3,?4,1789563194)",
                rusqlite::params![sid, node, parent, json],
            )
            .unwrap();
        };
        insert(&c, "fossil-echium", 0, None, r#"{"message_id":"m0","role":"system","content":"sysprompt"}"#);
        insert(&c, "fossil-echium", 1, Some(0), r#"{"message_id":"m1","role":"user","content":"Hi"}"#);
        // The forest stores every assistant message twice under one message_id.
        let msg = assistant("fef25c5a-c81a-4c37-99c8-91662adca801");
        insert(&c, "fossil-echium", 22, Some(1), &msg);
        insert(&c, "fossil-echium", 23, Some(1), &msg);
        // Interrupted generation: an assistant row the provider never reported.
        insert(
            &c,
            "fossil-echium",
            24,
            Some(1),
            r#"{"message_id":"m24","role":"assistant","content":"...","metadata":{"metrics":{"input_tokens":0,"output_tokens":0,"cache_read_tokens":0,"cache_creation_tokens":null},"created_at":"2026-09-16T12:54:00Z","generation_model":"swe-1-6-slow"}}"#,
        );
    }

    #[test]
    fn counts_each_generation_once() {
        let home = test_home("devin");
        let store = test_store("devin");
        seed(&home);
        assert_eq!(collect(&store, &home).unwrap(), 1);
        type Row = (String, String, i64, i64, i64, i64, i64, Option<i64>, Option<i64>, String);
        let row: Row = store
            .conn()
            .query_row(
                "SELECT source_event_id, COALESCE(project,''), input_tokens, output_tokens,
                        cache_read_tokens, cache_write_tokens, ts, ttft_ms, duration_ms, COALESCE(model,'')
                 FROM usage_event",
                [],
                |r| {
                    Ok((
                        r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?, r.get(9)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(row.0, "fossil-echium:fef25c5a-c81a-4c37-99c8-91662adca801");
        assert_eq!(row.1, "/Users/jack");
        assert_eq!((row.2, row.3, row.4, row.5), (683, 95, 12064, 0));
        assert_eq!(row.6, 1_789_563_194_012); // metadata.created_at, not the flush-time row clock
        assert_eq!(row.7, Some(1438));
        assert_eq!(row.8, Some(1885));
        assert_eq!(row.9, "swe-1-6-slow");

        // Re-sync inside the overlap window: the twin and the re-read upsert,
        // never a second stored event.
        collect(&store, &home).unwrap();
        let n: i64 = store
            .conn()
            .query_row("SELECT COUNT(*) FROM usage_event", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn missing_table_is_quietly_skipped() {
        let home = test_home("devin-empty");
        let store = test_store("devin-empty");
        let db = home.join(".local/share/devin/cli/sessions.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        std::fs::write(&db, b"").unwrap();
        assert_eq!(collect(&store, &home).unwrap(), 0);
    }

    /// Wiping ~/.local/share/devin and starting over must not leave the
    /// row-id cursor above the fresh database forever: the reinstall's
    /// low row ids are re-read, and the surviving history keeps both eras.
    #[test]
    fn wiped_and_reinstalled_db_is_reread() {
        let home = test_home("devin-wipe");
        let store = test_store("devin-wipe");
        seed(&home);
        assert_eq!(collect(&store, &home).unwrap(), 1);

        // Fresh install: brand-new database whose row ids restart at 1.
        std::fs::remove_file(home.join(".local/share/devin/cli/sessions.db")).unwrap();
        let db = home.join(".local/share/devin/cli/sessions.db");
        let c = rusqlite::Connection::open(&db).unwrap();
        c.execute_batch(
            "CREATE TABLE sessions (
               id TEXT PRIMARY KEY, working_directory TEXT NOT NULL, backend_type TEXT NOT NULL,
               model TEXT NOT NULL, agent_mode TEXT NOT NULL, created_at INTEGER NOT NULL,
               last_activity_at INTEGER NOT NULL);
             CREATE TABLE message_nodes (
               row_id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL,
               node_id INTEGER NOT NULL, parent_node_id INTEGER, chat_message TEXT NOT NULL,
               created_at INTEGER NOT NULL);
             INSERT INTO sessions VALUES ('delta-fjord', '/Users/jack/new', 'windsurf',
               'swe-1-6-slow', 'normal', 1789651200, 1789651210);",
        )
        .unwrap();
        let mut fresh = assistant("11111111-2222-3333-4444-555555555555");
        fresh = fresh.replace("2026-09-16T12:53:14.012948Z", "2026-09-17T09:20:00.000000Z");
        c.execute(
            "INSERT INTO message_nodes (session_id, node_id, parent_node_id, chat_message, created_at)
             VALUES ('delta-fjord', 0, NULL, ?1, 1789651210)",
            rusqlite::params![fresh],
        )
        .unwrap();

        // The pre-wipe event is still there (history lives here), and the new
        // one landed despite its row id being far below the old watermark.
        assert_eq!(collect(&store, &home).unwrap(), 1);
        let count = |sql: &str| -> i64 {
            store.conn().query_row(sql, [], |r| r.get(0)).unwrap()
        };
        // 2 stored events total, 1 of them from the reinstalled database.
        assert_eq!(count("SELECT COUNT(*) FROM usage_event"), 2);
        assert_eq!(count("SELECT COUNT(*) FROM usage_event WHERE session_id='delta-fjord'"), 1);
    }
}
