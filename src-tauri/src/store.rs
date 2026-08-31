use rusqlite::{params, Connection, OpenFlags};
use std::path::Path;

use crate::models::{ModelAlias, Source, UsageEvent};
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
    estimated INTEGER NOT NULL DEFAULT 0,
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
CREATE TABLE IF NOT EXISTS hidden_model (
    name TEXT PRIMARY KEY
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
        // CREATE TABLE IF NOT EXISTS leaves an existing table alone, so a column added
        // after a release has to be added here too. Errors are ignored on purpose: the
        // only one this can raise is "duplicate column name", which means it already ran.
        let _ = conn.execute(
            "ALTER TABLE usage_event ADD COLUMN estimated INTEGER NOT NULL DEFAULT 0",
            [],
        );
        Ok(Self { conn })
    }

    pub fn insert_events(&self, events: &[UsageEvent]) -> rusqlite::Result<usize> {
        if events.is_empty() {
            return Ok(0);
        }
        let mut stmt = self.conn.prepare_cached(
            "INSERT INTO usage_event (source, source_event_id, ts, session_id, project, provider, model,
                input_tokens, output_tokens, reasoning_tokens, cache_read_tokens, cache_write_tokens,
                duration_ms, ttft_ms, is_subagent, cost_usd, estimated)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)
             ON CONFLICT(source, source_event_id) DO UPDATE SET
                ts=excluded.ts, session_id=excluded.session_id, project=excluded.project,
                provider=excluded.provider, model=excluded.model,
                input_tokens=excluded.input_tokens, output_tokens=excluded.output_tokens,
                reasoning_tokens=excluded.reasoning_tokens,
                cache_read_tokens=excluded.cache_read_tokens, cache_write_tokens=excluded.cache_write_tokens,
                duration_ms=excluded.duration_ms, ttft_ms=excluded.ttft_ms,
                is_subagent=excluded.is_subagent, cost_usd=excluded.cost_usd,
                estimated=excluded.estimated",
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
                e.estimated as i64,
            ])?;
        }
        Ok(n)
    }

    /// Recompute `cost_usd` for every stored event against the current bundled
    /// pricing table. Called on startup when the embedded pricing fingerprint
    /// changes so history reflects updated list prices.
    pub fn reprice_all(&self) -> rusqlite::Result<usize> {
        // (id, source, model, input, output, reasoning, cache_read, cache_write)
        type Row = (i64, String, Option<String>, i64, i64, Option<i64>, i64, i64);
        let rows: Vec<Row> = {
            let mut stmt = self.conn.prepare_cached(
                "SELECT id, source, model, input_tokens, output_tokens, reasoning_tokens,
                        cache_read_tokens, cache_write_tokens FROM usage_event",
            )?;
            let q = stmt.query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                ))
            })?;
            q.collect::<Result<Vec<_>, _>>()?
        };
        let tx = self.conn.unchecked_transaction()?;
        let mut n = 0usize;
        for (id, source_s, model, input, output, reasoning, cr, cw) in rows {
            // Unknown sources behave like ZCode: no reasoning-token surcharge.
            let source = Source::from_str(&source_s).unwrap_or(Source::Zcode);
            let cost =
                pricing::cost_for(source, model.as_deref(), input, output, reasoning, cr, cw);
            n += tx.execute(
                "UPDATE usage_event SET cost_usd = ?2 WHERE id = ?1",
                params![id, cost],
            )?;
        }
        tx.commit()?;
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

    /// Drop every watermark whose key matches the LIKE `pattern` (e.g.
    /// "antigravity:%"). Collectors use this to force a full re-read after a
    /// parser fix: rows the old parser skipped still advanced their
    /// watermarks, and re-ingesting is safe because events upsert on
    /// (source, source_event_id).
    pub fn clear_watermarks(&self, pattern: &str) {
        let _ = self.conn.execute(
            "DELETE FROM ingest_state WHERE source LIKE ?1",
            params![pattern],
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

    pub fn get_raw_models(&self) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT DISTINCT model FROM usage_event WHERE model IS NOT NULL AND model <> '' ORDER BY model")?;
        let rows = stmt
            .query_map([], |r| r.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
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

    /// Rename a model display name: sets `current_name` to display as `new_name`.
    /// Pure query-time alias; raw `usage_event` rows are completely untouched.
    /// If `current_name == new_name`, removes any existing alias (reverting to default).
    /// If `current_name` was previously a canonical name for other aliases, repoints them to `new_name`.
    pub fn rename_model(&self, current_name: &str, new_name: &str) -> Result<(), String> {
        let current = current_name.trim();
        let target = new_name.trim();
        if current.is_empty() {
            return Err("current model name cannot be empty".to_string());
        }
        if target.is_empty() {
            return Err("new model name cannot be empty".to_string());
        }
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| format!("begin rename transaction: {e}"))?;

        if current == target {
            // Reverting to default / removing alias
            tx.execute("DELETE FROM model_alias WHERE alias = ?1", params![current])
                .map_err(|e| format!("remove alias: {e}"))?;
        } else {
            // Target must not have a self-alias
            tx.execute("DELETE FROM model_alias WHERE alias = ?1", params![target])
                .map_err(|e| format!("clear target self-alias: {e}"))?;

            // If `current` was a canonical name for other aliases, repoint them to `target`
            tx.execute(
                "UPDATE model_alias SET canonical = ?1 WHERE canonical = ?2 AND alias <> ?1",
                params![target, current],
            )
            .map_err(|e| format!("repoint aliases: {e}"))?;

            // Set current -> target
            tx.execute(
                "INSERT INTO model_alias(alias, canonical) VALUES(?1, ?2)
                 ON CONFLICT(alias) DO UPDATE SET canonical = excluded.canonical",
                params![current, target],
            )
            .map_err(|e| format!("set alias: {e}"))?;
        }

        tx.commit().map_err(|e| format!("commit rename: {e}"))?;
        Ok(())
    }

    pub fn get_hidden_models(&self) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT name FROM hidden_model ORDER BY name")?;
        let rows = stmt
            .query_map([], |r| r.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Record `names` as hidden so they are excluded from every aggregate.
    /// Idempotent; existing entries are left untouched.
    pub fn hide_models(&self, names: &[String]) -> Result<usize, String> {
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| format!("begin hide transaction: {e}"))?;
        let mut n = 0usize;
        let mut seen = std::collections::HashSet::new();
        for name in names {
            if !seen.insert(name.clone()) {
                continue;
            }
            n += tx
                .execute(
                    "INSERT INTO hidden_model(name) VALUES(?1) ON CONFLICT(name) DO NOTHING",
                    params![name],
                )
                .map_err(|e| format!("hide model: {e}"))?;
        }
        tx.commit().map_err(|e| format!("commit hide: {e}"))?;
        Ok(n)
    }

    pub fn unhide_model(&self, name: &str) -> rusqlite::Result<usize> {
        self.conn
            .execute("DELETE FROM hidden_model WHERE name = ?1", params![name])
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

    #[test]
    fn hide_round_trip() {
        let store = test_store();
        assert!(store.get_hidden_models().unwrap().is_empty());

        store.hide_models(&["codex-auto-review".into()]).unwrap();
        assert_eq!(store.get_hidden_models().unwrap(), vec!["codex-auto-review"]);

        // Re-hiding is a no-op, and multiple names are kept sorted.
        store.hide_models(&["codex-auto-review".into(), "gpt-5".into()]).unwrap();
        assert_eq!(
            store.get_hidden_models().unwrap(),
            vec!["codex-auto-review", "gpt-5"]
        );

        store.unhide_model("codex-auto-review").unwrap();
        assert_eq!(store.get_hidden_models().unwrap(), vec!["gpt-5"]);
    }

    #[test]
    fn reprice_all_recomputes_stored_costs() {
        let store = test_store();
        let e = UsageEvent {
            source: Source::Zcode,
            source_event_id: "r1".into(),
            ts: 0,
            session_id: None,
            project: None,
            provider: None,
            model: Some("claude-sonnet-4.5".into()),
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
            reasoning_tokens: None,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            duration_ms: None,
            ttft_ms: None,
            is_subagent: false,
            estimated: false,
        };
        store.insert_events(&[e]).unwrap();
        // Insert priced it at $18 (1M in + 1M out at $3/$15).
        let before: f64 = store
            .conn()
            .query_row("SELECT cost_usd FROM usage_event", [], |r| r.get(0))
            .unwrap();
        assert!((before - 18.0).abs() < 1e-9);

        // Sabotage the stored cost, then reprice_all restores it.
        store.conn().execute("UPDATE usage_event SET cost_usd = 1.0", []).unwrap();
        assert_eq!(store.reprice_all().unwrap(), 1);
        let after: f64 = store
            .conn()
            .query_row("SELECT cost_usd FROM usage_event", [], |r| r.get(0))
            .unwrap();
        assert!((after - 18.0).abs() < 1e-9);
    }

    #[test]
    fn rename_round_trip() {
        let store = test_store();
        // Rename raw model to custom name
        store
            .rename_model("deepseek-v4-flash:0731", "DeepSeek V4 Flash")
            .unwrap();
        let aliases = store.get_model_aliases().unwrap();
        assert_eq!(aliases.len(), 1);
        assert_eq!(aliases[0].alias, "deepseek-v4-flash:0731");
        assert_eq!(aliases[0].canonical, "DeepSeek V4 Flash");

        // Renaming to a new display name updates the alias
        store
            .rename_model("deepseek-v4-flash:0731", "DeepSeek Flash")
            .unwrap();
        let aliases = store.get_model_aliases().unwrap();
        assert_eq!(aliases.len(), 1);
        assert_eq!(aliases[0].canonical, "DeepSeek Flash");

        // Renaming back to itself (raw name) removes the alias
        store
            .rename_model("deepseek-v4-flash:0731", "deepseek-v4-flash:0731")
            .unwrap();
        assert!(store.get_model_aliases().unwrap().is_empty());
    }

    #[test]
    fn get_raw_models_returns_distinct_models() {
        let store = test_store();
        let e1 = UsageEvent {
            source: Source::Zcode,
            source_event_id: "r1".into(),
            ts: 1000,
            session_id: None,
            project: None,
            provider: None,
            model: Some("gpt-4o".into()),
            input_tokens: 100,
            output_tokens: 100,
            reasoning_tokens: None,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            duration_ms: None,
            ttft_ms: None,
            is_subagent: false,
            estimated: false,
        };
        let e2 = UsageEvent {
            source: Source::Zcode,
            source_event_id: "r2".into(),
            ts: 2000,
            session_id: None,
            project: None,
            provider: None,
            model: Some("gpt-4o-2024-08-06".into()),
            input_tokens: 100,
            output_tokens: 100,
            reasoning_tokens: None,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            duration_ms: None,
            ttft_ms: None,
            is_subagent: false,
            estimated: false,
        };
        let e3 = UsageEvent {
            source: Source::Zcode,
            source_event_id: "r3".into(),
            ts: 3000,
            session_id: None,
            project: None,
            provider: None,
            model: Some("gpt-4o".into()),
            input_tokens: 100,
            output_tokens: 100,
            reasoning_tokens: None,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            duration_ms: None,
            ttft_ms: None,
            is_subagent: false,
            estimated: false,
        };
        let e4 = UsageEvent {
            source: Source::Zcode,
            source_event_id: "r4".into(),
            ts: 4000,
            session_id: None,
            project: None,
            provider: None,
            model: None,
            input_tokens: 100,
            output_tokens: 100,
            reasoning_tokens: None,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            duration_ms: None,
            ttft_ms: None,
            is_subagent: false,
            estimated: false,
        };
        store.insert_events(&[e1, e2, e3, e4]).unwrap();

        let raw = store.get_raw_models().unwrap();
        assert_eq!(raw, vec!["gpt-4o", "gpt-4o-2024-08-06"]);
    }
}
