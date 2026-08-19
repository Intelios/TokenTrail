use rusqlite::{params, Connection, OpenFlags};
use std::path::Path;

use crate::models::UsageEvent;
use crate::pricing;

/// TokenTrail's own long-term store. Append-mostly: rows are keyed by
/// (source, source_event_id) so re-ingesting harness data is always safe,
/// and upserts let late-finalized token counts (e.g. ZCode) land correctly.
pub struct Store {
    conn: Connection,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS usage_event (
    id INTEGER PRIMARY KEY,
    source TEXT NOT NULL,
    source_event_id TEXT NOT NULL,
    ts INTEGER NOT NULL,
    session_id TEXT,
    project TEXT,
    provider TEXT,
    model TEXT,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    reasoning_tokens INTEGER,
    cache_read_tokens INTEGER NOT NULL DEFAULT 0,
    cache_write_tokens INTEGER NOT NULL DEFAULT 0,
    duration_ms INTEGER,
    ttft_ms INTEGER,
    is_subagent INTEGER NOT NULL DEFAULT 0,
    cost_usd REAL,
    UNIQUE(source, source_event_id)
);
CREATE INDEX IF NOT EXISTS idx_usage_ts ON usage_event(ts);
CREATE INDEX IF NOT EXISTS idx_usage_source_ts ON usage_event(source, ts);
CREATE INDEX IF NOT EXISTS idx_usage_session ON usage_event(source, session_id);
CREATE TABLE IF NOT EXISTS ingest_state (
    source TEXT NOT NULL,
    path TEXT NOT NULL,
    offset INTEGER NOT NULL DEFAULT 0,
    watermark INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(source, path)
);
"#;

impl Store {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    pub fn insert_events(&self, events: &[UsageEvent]) -> rusqlite::Result<usize> {
        if events.is_empty() {
            return Ok(0);
        }
        let mut stmt = self.conn.prepare_cached(
            "INSERT INTO usage_event (source, source_event_id, ts, session_id, project, provider, model,
                input_tokens, output_tokens, reasoning_tokens, cache_read_tokens, cache_write_tokens,
                duration_ms, ttft_ms, is_subagent, cost_usd)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)
             ON CONFLICT(source, source_event_id) DO UPDATE SET
                ts=excluded.ts, session_id=excluded.session_id, project=excluded.project,
                provider=excluded.provider, model=excluded.model,
                input_tokens=excluded.input_tokens, output_tokens=excluded.output_tokens,
                reasoning_tokens=excluded.reasoning_tokens,
                cache_read_tokens=excluded.cache_read_tokens, cache_write_tokens=excluded.cache_write_tokens,
                duration_ms=excluded.duration_ms, ttft_ms=excluded.ttft_ms,
                is_subagent=excluded.is_subagent, cost_usd=excluded.cost_usd",
        )?;
        let mut n = 0;
        for e in events {
            let cost = pricing::cost_usd(e.model.as_deref(), e);
            n += stmt.execute(params![
                e.source.as_str(),
                e.source_event_id,
                e.ts,
                e.session_id,
                e.project,
                e.provider,
                e.model,
                e.input_tokens,
                e.output_tokens,
                e.reasoning_tokens,
                e.cache_read_tokens,
                e.cache_write_tokens,
                e.duration_ms,
                e.ttft_ms,
                e.is_subagent as i64,
                cost,
            ])?;
        }
        Ok(n)
    }

    pub fn get_offset(&self, source: &str, path: &str) -> Option<u64> {
        self.conn
            .query_row(
                "SELECT offset FROM ingest_state WHERE source=?1 AND path=?2",
                params![source, path],
                |r| r.get::<_, i64>(0),
            )
            .ok()
            .map(|v| v as u64)
    }

    pub fn set_offset(&self, source: &str, path: &str, offset: u64) {
        let _ = self.conn.execute(
            "INSERT INTO ingest_state(source,path,offset,updated_at) VALUES(?1,?2,?3,strftime('%s','now'))
             ON CONFLICT(source,path) DO UPDATE SET offset=excluded.offset, updated_at=excluded.updated_at",
            params![source, path, offset as i64],
        );
    }

    pub fn get_watermark(&self, key: &str) -> i64 {
        self.conn
            .query_row(
                "SELECT watermark FROM ingest_state WHERE source=?1 AND path=''",
                params![key],
                |r| r.get(0),
            )
            .unwrap_or(0)
    }

    pub fn set_watermark(&self, key: &str, watermark: i64) {
        let _ = self.conn.execute(
            "INSERT INTO ingest_state(source,path,watermark,updated_at) VALUES(?1,'',?2,strftime('%s','now'))
             ON CONFLICT(source,path) DO UPDATE SET watermark=excluded.watermark, updated_at=excluded.updated_at",
            params![key, watermark],
        );
    }

    pub fn latest_session_model(&self, source: &str, session_id: &str) -> Option<String> {
        self.conn
            .query_row(
                "SELECT model FROM usage_event WHERE source=?1 AND session_id=?2 AND model IS NOT NULL
                 ORDER BY ts DESC LIMIT 1",
                params![source, session_id],
                |r| r.get(0),
            )
            .ok()
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}

/// Open a harness-owned SQLite database strictly read-only.
pub fn open_readonly(path: &Path) -> rusqlite::Result<Connection> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
        | OpenFlags::SQLITE_OPEN_NO_MUTEX
        | OpenFlags::SQLITE_OPEN_URI;
    match Connection::open_with_flags(path, flags) {
        Ok(c) => Ok(c),
        Err(_) => Connection::open_with_flags(format!("file:{}?immutable=1", path.display()), flags),
    }
}
