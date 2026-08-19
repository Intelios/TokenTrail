use crate::models::{Source, UsageEvent};
use crate::store::{open_readonly, Store};
use std::path::Path;

/// ZCode keeps a proper relational usage ledger in ~/.zcode/cli/db/db.sqlite.
/// Rows can finalize after we first read them (in-flight requests), so we
/// re-read a 5-minute overlap window and rely on upserts.
const OVERLAP_MS: i64 = 5 * 60 * 1000;

pub fn collect(store: &Store, home: &Path) -> Result<usize, String> {
    let db = home.join(".zcode/cli/db/db.sqlite");
    if !db.exists() {
        return Ok(0);
    }
    let conn = open_readonly(&db).map_err(|e| format!("open zcode db: {e}"))?;
    let watermark = store.get_watermark("zcode").saturating_sub(OVERLAP_MS);

    let mut stmt = conn
        .prepare(
            "SELECT m.id, m.session_id, s.directory, m.model_id, m.started_at, m.duration_ms,
                    m.time_to_first_token_ms, m.input_tokens, m.output_tokens, m.reasoning_tokens,
                    m.cache_creation_input_tokens, m.cache_read_input_tokens
             FROM model_usage m
             LEFT JOIN session s ON s.id = m.session_id
             WHERE m.started_at > ?1
               AND (m.input_tokens + m.output_tokens + m.cache_creation_input_tokens
                    + m.cache_read_input_tokens) > 0
             ORDER BY m.started_at",
        )
        .map_err(|e| format!("zcode prepare: {e}"))?;

    let rows = stmt
        .query_map([watermark], |r| {
            Ok(UsageEvent {
                source: Source::Zcode,
                source_event_id: r.get::<_, String>(0)?,
                session_id: r.get::<_, Option<String>>(1)?,
                project: r.get::<_, Option<String>>(2)?,
                provider: None,
                model: r.get::<_, Option<String>>(3)?,
                ts: r.get::<_, i64>(4)?,
                duration_ms: r.get::<_, Option<i64>>(5)?,
                ttft_ms: r.get::<_, Option<i64>>(6)?,
                input_tokens: r.get::<_, i64>(7)?,
                output_tokens: r.get::<_, i64>(8)?,
                reasoning_tokens: r.get::<_, Option<i64>>(9)?,
                cache_write_tokens: r.get::<_, i64>(10)?,
                cache_read_tokens: r.get::<_, i64>(11)?,
                is_subagent: false,
            })
        })
        .map_err(|e| format!("zcode query: {e}"))?;

    let mut events = Vec::new();
    let mut max_ts = 0i64;
    for row in rows {
        let e = row.map_err(|e| format!("zcode row: {e}"))?;
        max_ts = max_ts.max(e.ts);
        events.push(e);
    }
    let processed = store.insert_events(&events).map_err(|e| format!("zcode insert: {e}"))?;
    if max_ts > 0 {
        store.set_watermark("zcode", max_ts);
    }
    Ok(processed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collectors::{test_home, test_store};
    use rusqlite::Connection;

    fn seed(home: &Path) {
        let db = home.join(".zcode/cli/db/db.sqlite");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let c = Connection::open(&db).unwrap();
        c.execute_batch(
            "CREATE TABLE session (id TEXT PRIMARY KEY, directory TEXT NOT NULL);
             CREATE TABLE model_usage (
               id TEXT PRIMARY KEY, session_id TEXT NOT NULL REFERENCES session(id),
               model_id TEXT, started_at INTEGER, duration_ms INTEGER,
               time_to_first_token_ms INTEGER, input_tokens INTEGER DEFAULT 0,
               output_tokens INTEGER DEFAULT 0, reasoning_tokens INTEGER DEFAULT 0,
               cache_creation_input_tokens INTEGER DEFAULT 0, cache_read_input_tokens INTEGER DEFAULT 0
             );
             INSERT INTO session VALUES ('s1', '/Users/jack/proj');
             INSERT INTO model_usage (id, session_id, model_id, started_at, input_tokens, output_tokens, cache_creation_input_tokens, cache_read_input_tokens)
               VALUES ('m1', 's1', 'deepseek-v4-flash:0731-cloud', 1754000000000, 100, 200, 300, 400);
             INSERT INTO model_usage (id, session_id, model_id, started_at, input_tokens)
               VALUES ('m2', 's1', 'deepseek-v4-flash:0731-cloud', 1754000001000, 0);",
        )
        .unwrap();
    }

    #[test]
    fn ingests_nonzero_rows_once() {
        let home = test_home("zcode");
        let store = test_store("zcode");
        seed(&home);
        let n = collect(&store, &home).unwrap();
        assert_eq!(n, 1);
        let count: i64 = store
            .conn()
            .query_row("SELECT COUNT(*) FROM usage_event", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
        let (proj, model, ttft): (String, String, Option<i64>) = store
            .conn()
            .query_row("SELECT project, model, ttft_ms FROM usage_event", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .unwrap();
        assert_eq!(proj, "/Users/jack/proj");
        assert!(model.starts_with("deepseek"));
        assert!(ttft.is_none());
        // second pass (inside overlap): same rows, still exactly one stored event
        collect(&store, &home).unwrap();
        let count: i64 = store
            .conn()
            .query_row("SELECT COUNT(*) FROM usage_event", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }
}
