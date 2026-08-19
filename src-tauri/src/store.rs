use rusqlite::{params, Connection, OpenFlags};
use std::path::Path;

use crate::models::{ModelAlias, UsageEvent};
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
CREATE TABLE IF NOT EXISTS model_alias (
    alias TEXT PRIMARY KEY,
    canonical TEXT NOT NULL
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

    pub fn get_model_aliases(&self) -> rusqlite::Result<Vec<ModelAlias>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT alias, canonical FROM model_alias ORDER BY canonical, alias")?;
        let rows = stmt
            .query_map([], |r| {
                Ok(ModelAlias {
                    alias: r.get(0)?,
                    canonical: r.get(1)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Merge `names` into one display name `canonical` (which must be one of the
    /// names). Existing aliases whose canonical is being absorbed are repointed to
    /// `canonical` so the table never forms alias -> alias chains. Idempotent.
    pub fn merge_models(&self, names: &[String], canonical: &str) -> Result<usize, String> {
        if names.len() < 2 {
            return Err("merge needs at least two model names".to_string());
        }
        if !names.iter().any(|n| n == canonical) {
            return Err("canonical name must be one of the merged names".to_string());
        }
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| format!("begin merge transaction: {e}"))?;
        // A canonical that was previously an alias must not keep its own row.
        tx.execute("DELETE FROM model_alias WHERE alias = ?1", params![canonical])
            .map_err(|e| format!("clear canonical self-alias: {e}"))?;
        let mut n = 0usize;
        let mut seen = std::collections::HashSet::new();
        for name in names.iter().filter(|n| *n != canonical) {
            if !seen.insert(name.clone()) {
                continue;
            }
            // Existing aliases pointing at an absorbed name follow it to the new canonical.
            n += tx
                .execute(
                    "UPDATE model_alias SET canonical = ?1 WHERE canonical = ?2 AND alias <> ?1",
                    params![canonical, name],
                )
                .map_err(|e| format!("repoint aliases: {e}"))?;
            n += tx
                .execute(
                    "INSERT INTO model_alias(alias, canonical) VALUES(?1, ?2)
                     ON CONFLICT(alias) DO UPDATE SET canonical = excluded.canonical",
                    params![name, canonical],
                )
                .map_err(|e| format!("set alias: {e}"))?;
        }
        tx.commit().map_err(|e| format!("commit merge: {e}"))?;
        Ok(n)
    }

    pub fn remove_model_alias(&self, alias: &str) -> rusqlite::Result<usize> {
        self.conn
            .execute("DELETE FROM model_alias WHERE alias = ?1", params![alias])
    }

    pub fn remove_aliases_for(&self, canonical: &str) -> rusqlite::Result<usize> {
        self.conn
            .execute("DELETE FROM model_alias WHERE canonical = ?1", params![canonical])
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store() -> Store {
        Store::open(std::path::Path::new(":memory:")).unwrap()
    }

    #[test]
    fn merge_round_trip() {
        let store = test_store();
        store
            .merge_models(&["GLM-5.3".into(), "glm-5.3".into()], "GLM-5.3")
            .unwrap();
        let aliases = store.get_model_aliases().unwrap();
        assert_eq!(aliases.len(), 1);
        assert_eq!(aliases[0], ModelAlias { alias: "glm-5.3".into(), canonical: "GLM-5.3".into() });

        store.remove_aliases_for("GLM-5.3").unwrap();
        assert!(store.get_model_aliases().unwrap().is_empty());
    }

    #[test]
    fn merge_repoints_existing_aliases() {
        let store = test_store();
        // A and B become group "B", then that group is merged under "C":
        // A -> B must follow the group and become A -> C (no chains).
        store.merge_models(&["A".into(), "B".into()], "B").unwrap();
        store.merge_models(&["B".into(), "C".into()], "C").unwrap();
        let mut map: std::collections::HashMap<String, String> = store
            .get_model_aliases()
            .unwrap()
            .into_iter()
            .map(|a| (a.alias, a.canonical))
            .collect();
        assert_eq!(map.remove("A"), Some("C".into()));
        assert_eq!(map.remove("B"), Some("C".into()));
        assert!(map.is_empty());
    }

    #[test]
    fn merge_validates_input() {
        let store = test_store();
        // need at least two names
        assert!(store.merge_models(&["A".into()], "A").is_err());
        // canonical must be one of the names
        assert!(store.merge_models(&["A".into(), "B".into()], "C").is_err());
    }

    #[test]
    fn merge_is_idempotent() {
        let store = test_store();
        store.merge_models(&["A".into(), "B".into()], "A").unwrap();
        store.merge_models(&["A".into(), "B".into()], "A").unwrap();
        let aliases = store.get_model_aliases().unwrap();
        assert_eq!(aliases.len(), 1);
        assert_eq!(aliases[0].alias, "B");
    }

    #[test]
    fn remove_single_alias() {
        let store = test_store();
        store.merge_models(&["A".into(), "B".into(), "C".into()], "A").unwrap();
        store.remove_model_alias("B").unwrap();
        let aliases = store.get_model_aliases().unwrap();
        assert_eq!(aliases.len(), 1);
        assert_eq!(aliases[0].alias, "C");
        assert_eq!(aliases[0].canonical, "A");
    }
}
