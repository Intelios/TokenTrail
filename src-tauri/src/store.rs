use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use crate::models::{ModelAlias, ProjectAlias, ProjectColor, ProjectStack, Source, UsageEvent};
use crate::pricing;

/// How long a connection waits for a locked database before giving up. With
/// readers and writer on separate WAL connections the remaining contention is
/// a checkpoint racing a query, which passes quickly.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Read connections kept alive between queries; beyond this they are opened
/// per call and dropped when their guard goes out of scope.
const MAX_POOLED_READERS: usize = 4;

/// WackChatter delivery provenance: transcript rank 1, retained ledger rank 2.
#[derive(Default)]
pub struct WackContext {
    pub rank: i64,
    pub library_id: Option<String>,
    pub character_id: Option<String>,
    pub legacy_id: Option<String>,
}

/// TokenTrail's own long-term store. Append-mostly: rows are keyed by
/// (source, source_event_id) so re-ingesting harness data is always safe,
/// and upserts let late-finalized token counts (e.g. ZCode) land correctly.
///
/// One writer connection plus a small pool of readers. The database is in
/// WAL mode, so a query never queues behind a sync's write batches — the
/// background sync can commit while UI reads run right past it.
pub struct Store {
    writer: Mutex<Connection>,
    path: PathBuf,
    readers: Mutex<Vec<Connection>>,
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
    provider_name TEXT,
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
    purpose TEXT,
    outcome TEXT,
    workspace TEXT,
    subagent_id TEXT,
    mode TEXT,
    UNIQUE(source, source_event_id)
);
CREATE INDEX IF NOT EXISTS idx_usage_ts ON usage_event(ts);
CREATE INDEX IF NOT EXISTS idx_usage_source_ts ON usage_event(source, ts);
CREATE INDEX IF NOT EXISTS idx_usage_session ON usage_event(source, session_id, ts DESC);
CREATE INDEX IF NOT EXISTS idx_usage_model ON usage_event(model, ts);
CREATE INDEX IF NOT EXISTS idx_usage_project ON usage_event(project, ts);
CREATE TABLE IF NOT EXISTS wackchatter_library (
    path TEXT PRIMARY KEY, library_id TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS wackchatter_provenance (
    event_id TEXT PRIMARY KEY, rank INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS wackchatter_membership (
    event_id TEXT NOT NULL, library_id TEXT NOT NULL, session_id TEXT NOT NULL DEFAULT '',
    character_id TEXT NOT NULL DEFAULT '', rank INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY(event_id, library_id, session_id, character_id)
);
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
CREATE TABLE IF NOT EXISTS project_color (
    project TEXT PRIMARY KEY,
    color TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS project_alias (
    alias TEXT PRIMARY KEY,
    canonical TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS project_stack (
    project TEXT NOT NULL,
    tag TEXT NOT NULL,
    ord INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(project, tag)
);
"#;

impl Store {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(SCHEMA)?;
        for column in [
            "purpose",
            "outcome",
            "workspace",
            "subagent_id",
            "provider_name",
            "mode",
        ] {
            let exists = conn
                .prepare("SELECT name FROM pragma_table_info('usage_event') WHERE name=?1")?
                .exists([column])?;
            if !exists {
                conn.execute(
                    &format!("ALTER TABLE usage_event ADD COLUMN {column} TEXT"),
                    [],
                )?;
            }
        }
        // CREATE TABLE IF NOT EXISTS leaves an existing table alone, so a column added
        // after a release has to be added here too. Errors are ignored on purpose: the
        // only one this can raise is "duplicate column name", which means it already ran.
        let _ = conn.execute(
            "ALTER TABLE usage_event ADD COLUMN estimated INTEGER NOT NULL DEFAULT 0",
            [],
        );
        if !conn
            .prepare(
                "SELECT name FROM pragma_table_info('wackchatter_membership') WHERE name='rank'",
            )?
            .exists([])?
        {
            conn.execute(
                "ALTER TABLE wackchatter_membership ADD COLUMN rank INTEGER NOT NULL DEFAULT 1",
                [],
            )?;
        }
        // Repair estimates priced by an older reprice_all implementation. Idempotent,
        // and independent of whether the bundled pricing fingerprint changed.
        conn.execute(
            "UPDATE usage_event SET cost_usd=NULL WHERE estimated=1 AND cost_usd IS NOT NULL",
            [],
        )?;
        // Ensure new indexes exist on databases created before they were added to SCHEMA.
        let _ = conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_usage_model ON usage_event(model, ts);
             CREATE INDEX IF NOT EXISTS idx_usage_project ON usage_event(project, ts);",
        );
        // Existing databases created before idx_usage_session included `ts DESC` need to be
        // upgraded so latest_session_model can use an index seek instead of sorting in memory.
        let needs_session_idx_rebuild = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='index' AND name='idx_usage_session'",
                [],
                |r| r.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten()
            .map(|sql| !sql.to_lowercase().contains("ts"))
            .unwrap_or(false);

        if needs_session_idx_rebuild {
            let _ = conn.execute_batch(
                "DROP INDEX IF EXISTS idx_usage_session;
                 CREATE INDEX IF NOT EXISTS idx_usage_session ON usage_event(source, session_id, ts DESC);",
            );
        }
        Ok(Self {
            writer: Mutex::new(conn),
            path: path.to_path_buf(),
            readers: Mutex::new(Vec::new()),
        })
    }

    /// The single write connection, for ingest state and all mutations. Poison
    /// is recovered from rather than propagated: a panicking thread must not
    /// turn the app's storage into a brick.
    fn writer(&self) -> MutexGuard<'_, Connection> {
        self.writer.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// A connection to run read-only queries against: a pooled reader when the
    /// store is file-backed, the writer connection itself for in-memory stores
    /// (a second connection to `:memory:` would be a different, empty database).
    pub fn read_conn(&self) -> ReadConn<'_> {
        if self.path == Path::new(":memory:") {
            return ReadConn::Writer(self.writer());
        }
        let pooled = self.readers.lock().unwrap_or_else(|p| p.into_inner()).pop();
        match pooled {
            Some(conn) => ReadConn::Pooled(PooledRead {
                conn: Some(conn),
                pool: &self.readers,
            }),
            None => match open_reader(&self.path) {
                Ok(conn) => ReadConn::Pooled(PooledRead {
                    conn: Some(conn),
                    pool: &self.readers,
                }),
                // Unreachable in practice (Store::open created the file), but a
                // query should degrade to the writer rather than fail outright.
                Err(_) => ReadConn::Writer(self.writer()),
            },
        }
    }

    pub fn remember_wackchatter_library(&self, path: &str, id: &str) -> rusqlite::Result<()> {
        self.writer().execute("INSERT INTO wackchatter_library(path,library_id) VALUES(?1,?2) ON CONFLICT(path) DO UPDATE SET library_id=excluded.library_id", params![path,id])?;
        Ok(())
    }
    pub fn wackchatter_library_id(&self, path: &str) -> Option<String> {
        self.read_conn()
            .query_row(
                "SELECT library_id FROM wackchatter_library WHERE path=?1",
                [path],
                |r| r.get(0),
            )
            .ok()
    }

    pub fn insert_events(&self, events: &[UsageEvent]) -> rusqlite::Result<usize> {
        self.insert_with_context(events, &[])
    }

    pub fn insert_wackchatter(
        &self,
        events: &[UsageEvent],
        contexts: &[WackContext],
    ) -> rusqlite::Result<usize> {
        self.insert_with_context(events, contexts)
    }

    fn insert_with_context(
        &self,
        events: &[UsageEvent],
        contexts: &[WackContext],
    ) -> rusqlite::Result<usize> {
        if events.is_empty() {
            return Ok(0);
        }
        // One transaction per batch. Each row used to commit on its own, so a
        // large sync spent most of its time in per-row WAL commits.
        let conn = self.writer();
        let tx = conn.unchecked_transaction()?;
        let mut n = 0;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO usage_event (source, source_event_id, ts, session_id, project, provider, provider_name, model,
                    input_tokens, output_tokens, reasoning_tokens, cache_read_tokens, cache_write_tokens,
                    duration_ms, ttft_ms, is_subagent, cost_usd, estimated, purpose, outcome, workspace, subagent_id, mode)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23)
                 ON CONFLICT(source, source_event_id) DO UPDATE SET
                    ts=excluded.ts, session_id=excluded.session_id, project=excluded.project,
                    provider=excluded.provider, provider_name=excluded.provider_name, model=excluded.model,
                    input_tokens=excluded.input_tokens, output_tokens=excluded.output_tokens,
                    reasoning_tokens=excluded.reasoning_tokens,
                    cache_read_tokens=excluded.cache_read_tokens, cache_write_tokens=excluded.cache_write_tokens,
                    duration_ms=excluded.duration_ms, ttft_ms=excluded.ttft_ms,
                    is_subagent=excluded.is_subagent, cost_usd=excluded.cost_usd,
                    estimated=excluded.estimated, purpose=excluded.purpose, outcome=excluded.outcome,
                    workspace=excluded.workspace, subagent_id=excluded.subagent_id, mode=excluded.mode",
            )?;
            for (index, incoming) in events.iter().enumerate() {
                let mut merged = incoming.clone();
                if let Some(ctx) = contexts.get(index) {
                    // Repair pre-library fallback identities in place, never duplicate them.
                    if let Some(old) = &ctx.legacy_id {
                        tx.execute("UPDATE OR IGNORE usage_event SET source_event_id=?1 WHERE source='wackchatter' AND source_event_id=?2", params![incoming.source_event_id, old])?;
                        tx.execute("DELETE FROM usage_event WHERE source='wackchatter' AND source_event_id=?1 AND EXISTS(SELECT 1 FROM usage_event WHERE source='wackchatter' AND source_event_id=?2)", params![old, incoming.source_event_id])?;
                    }
                    let previous = tx.query_row("SELECT ts,session_id,project,provider,provider_name,model,input_tokens,output_tokens,reasoning_tokens,cache_read_tokens,cache_write_tokens,duration_ms,ttft_ms,is_subagent,estimated,purpose,outcome,COALESCE((SELECT rank FROM wackchatter_provenance WHERE event_id=u.source_event_id),0) FROM usage_event u WHERE source='wackchatter' AND source_event_id=?1", [&incoming.source_event_id], |r| {
                        Ok((UsageEvent { source: Source::WackChatter, source_event_id: incoming.source_event_id.clone(), ts:r.get(0)?,session_id:r.get(1)?,project:r.get(2)?,provider:r.get(3)?,provider_name:r.get(4)?,model:r.get(5)?,input_tokens:r.get(6)?,output_tokens:r.get(7)?,reasoning_tokens:r.get(8)?,cache_read_tokens:r.get(9)?,cache_write_tokens:r.get(10)?,duration_ms:r.get(11)?,ttft_ms:r.get(12)?,is_subagent:r.get(13)?,estimated:r.get(14)?,purpose:r.get(15)?,outcome:r.get(16)?,workspace:None,subagent_id:None,mode:None },r.get::<_,i64>(17)?))
                    }).optional()?;
                    let mut rank = ctx.rank;
                    if let Some((old, old_rank)) = previous {
                        let keep_old = (!old.estimated && incoming.estimated)
                            || (old.estimated == incoming.estimated && old_rank > ctx.rank);
                        let (winner, loser) = if keep_old {
                            rank = old_rank;
                            (old, incoming.clone())
                        } else {
                            (incoming.clone(), old)
                        };
                        merged = winner;
                        merged.session_id = merged.session_id.or(loser.session_id);
                        merged.provider = merged.provider.or(loser.provider);
                        merged.provider_name = merged.provider_name.or(loser.provider_name);
                        merged.model = merged.model.or(loser.model);
                        merged.purpose = merged.purpose.or(loser.purpose);
                        merged.outcome = merged.outcome.or(loser.outcome);
                        merged.duration_ms = merged.duration_ms.or(loser.duration_ms);
                        merged.ttft_ms = merged.ttft_ms.or(loser.ttft_ms);
                        if merged.estimated == loser.estimated {
                            merged.reasoning_tokens =
                                merged.reasoning_tokens.or(loser.reasoning_tokens);
                        }
                    }
                    tx.execute("INSERT INTO wackchatter_provenance(event_id,rank) VALUES(?1,?2) ON CONFLICT(event_id) DO UPDATE SET rank=excluded.rank", params![incoming.source_event_id,rank])?;
                    if let Some(library) = &ctx.library_id {
                        tx.execute("INSERT INTO wackchatter_membership(event_id,library_id,session_id,character_id,rank) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(event_id,library_id,session_id,character_id) DO UPDATE SET rank=MAX(rank,excluded.rank)",params![incoming.source_event_id,library,incoming.session_id.as_deref().unwrap_or(""),ctx.character_id.as_deref().unwrap_or(""),ctx.rank])?;
                    }
                }
                let e = &merged;
                let cost = pricing::cost_usd(e.model.as_deref(), e);
                n += stmt.execute(params![
                    e.source.as_str(),
                    e.source_event_id,
                    e.ts,
                    e.session_id,
                    e.project,
                    e.provider,
                    e.provider_name,
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
                    e.purpose,
                    e.outcome,
                    e.workspace,
                    e.subagent_id,
                    e.mode,
                ])?;
            }
        }
        // Fold freshly seen folders into their project right away: a subdirectory
        // recorded mid-sync must never render as its own project.
        let mut projects = std::collections::HashSet::new();
        for e in events {
            if let Some(p) = &e.project {
                if !p.is_empty() {
                    projects.insert(p.clone());
                }
            }
        }
        for p in &projects {
            fold_project_alias(&tx, p)?;
        }
        tx.commit()?;
        Ok(n)
    }

    /// Recompute `cost_usd` for every stored event against the current bundled
    /// pricing table. Called on startup when the embedded pricing fingerprint
    /// changes so history reflects updated list prices.
    pub fn reprice_all(&self) -> rusqlite::Result<usize> {
        let conn = self.writer();
        // (id, source, model, input, output, reasoning, cache_read, cache_write)
        type Row = (i64, String, Option<String>, i64, i64, Option<i64>, i64, i64);
        let rows: Vec<Row> = {
            let mut stmt = conn.prepare_cached(
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
        let tx = conn.unchecked_transaction()?;
        let mut n = 0usize;
        for (id, source_s, model, input, output, reasoning, cr, cw) in rows {
            // Unknown sources behave like ZCode: no reasoning-token surcharge.
            let source = Source::from_str(&source_s).unwrap_or(Source::Zcode);
            let cost =
                pricing::cost_for(source, model.as_deref(), input, output, reasoning, cr, cw);
            n += tx.execute(
                "UPDATE usage_event SET cost_usd = CASE WHEN estimated=0 THEN ?2 END WHERE id = ?1",
                params![id, cost],
            )?;
        }
        tx.commit()?;
        Ok(n)
    }

    pub fn get_offset(&self, source: &str, path: &str) -> Option<u64> {
        let conn = self.writer();
        conn.query_row(
            "SELECT offset FROM ingest_state WHERE source=?1 AND path=?2",
            params![source, path],
            |r| r.get::<_, i64>(0),
        )
        .ok()
        .map(|v| v as u64)
    }

    pub fn set_offset(&self, source: &str, path: &str, offset: u64) {
        let conn = self.writer();
        let _ = conn.execute(
            "INSERT INTO ingest_state(source,path,offset,updated_at) VALUES(?1,?2,?3,strftime('%s','now'))
             ON CONFLICT(source,path) DO UPDATE SET offset=excluded.offset, updated_at=excluded.updated_at",
            params![source, path, offset as i64],
        );
    }

    /// Record that `path` is somewhere this source reads from, and list them back.
    ///
    /// For sources whose target can be swapped underneath them: a collector that only
    /// ever looks at wherever it is pointed *now* stops keeping the others current the
    /// moment the pointer moves. Remembering is what makes several of them one source.
    ///
    /// A path is never forgotten. One that has gone missing is skipped, not dropped —
    /// an external drive comes back, and a library on it should still be there when it
    /// does. The cost of keeping a genuinely dead entry is one `exists()` per sync.
    pub fn remember_path(&self, source: &str, path: &str) {
        let conn = self.writer();
        let _ = conn.execute(
            "INSERT OR IGNORE INTO ingest_state(source,path,updated_at)
             VALUES(?1,?2,strftime('%s','now'))",
            params![source, path],
        );
    }

    pub fn known_paths(&self, source: &str) -> Vec<String> {
        let conn = self.writer();
        let Ok(mut stmt) = conn
            .prepare("SELECT path FROM ingest_state WHERE source=?1 AND path<>'' ORDER BY path")
        else {
            return Vec::new();
        };
        stmt.query_map(params![source], |r| r.get(0))
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default()
    }

    pub fn get_watermark(&self, key: &str) -> i64 {
        let conn = self.writer();
        conn.query_row(
            "SELECT watermark FROM ingest_state WHERE source=?1 AND path=''",
            params![key],
            |r| r.get(0),
        )
        .unwrap_or(0)
    }

    pub fn set_watermark(&self, key: &str, watermark: i64) {
        let conn = self.writer();
        let _ = conn.execute(
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
        let conn = self.writer();
        let _ = conn.execute(
            "DELETE FROM ingest_state WHERE source LIKE ?1",
            params![pattern],
        );
    }

    pub fn latest_session_model(&self, source: &str, session_id: &str) -> Option<String> {
        let conn = self.writer();
        conn.query_row(
            "SELECT model FROM usage_event WHERE source=?1 AND session_id=?2 AND model IS NOT NULL
             ORDER BY ts DESC LIMIT 1",
            params![source, session_id],
            |r| r.get(0),
        )
        .ok()
    }

    pub fn get_raw_models(&self) -> rusqlite::Result<Vec<String>> {
        let conn = self.read_conn();
        let mut stmt = conn
            .prepare_cached("SELECT DISTINCT model FROM usage_event WHERE model IS NOT NULL AND model <> '' ORDER BY model")?;
        let rows = stmt
            .query_map([], |r| r.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn get_model_aliases(&self) -> rusqlite::Result<Vec<ModelAlias>> {
        let conn = self.read_conn();
        let mut stmt = conn
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
        let conn = self.writer();
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("begin merge transaction: {e}"))?;
        // A canonical that was previously an alias must not keep its own row.
        tx.execute(
            "DELETE FROM model_alias WHERE alias = ?1",
            params![canonical],
        )
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
        let conn = self.writer();
        conn.execute("DELETE FROM model_alias WHERE alias = ?1", params![alias])
    }

    pub fn remove_aliases_for(&self, canonical: &str) -> rusqlite::Result<usize> {
        let conn = self.writer();
        conn.execute(
            "DELETE FROM model_alias WHERE canonical = ?1",
            params![canonical],
        )
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
        let conn = self.writer();
        let tx = conn
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
        let conn = self.read_conn();
        let mut stmt = conn.prepare_cached("SELECT name FROM hidden_model ORDER BY name")?;
        let rows = stmt
            .query_map([], |r| r.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Record `names` as hidden so they are excluded from every aggregate.
    /// Idempotent; existing entries are left untouched.
    pub fn hide_models(&self, names: &[String]) -> Result<usize, String> {
        let conn = self.writer();
        let tx = conn
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
        let conn = self.writer();
        conn.execute("DELETE FROM hidden_model WHERE name = ?1", params![name])
    }

    pub fn get_project_colors(&self) -> rusqlite::Result<Vec<ProjectColor>> {
        let conn = self.read_conn();
        let mut stmt =
            conn.prepare_cached("SELECT project, color FROM project_color ORDER BY project")?;
        let rows = stmt
            .query_map([], |r| {
                Ok(ProjectColor {
                    project: r.get(0)?,
                    color: r.get(1)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Pin `project` to `color` (a `#rrggbb` hex, stored lowercase). Purely a display
    /// annotation: no aggregate reads this table. "unknown" is the bucket for events
    /// with no project, not a project, so it can't be colored.
    pub fn set_project_color(&self, project: &str, color: &str) -> Result<(), String> {
        let project = project.trim();
        if project.is_empty() || project == "unknown" {
            return Err(format!("cannot color project {project:?}"));
        }
        let color = normalize_hex(color).ok_or_else(|| format!("invalid color {color:?}"))?;
        let conn = self.writer();
        conn.execute(
            "INSERT INTO project_color(project, color) VALUES(?1, ?2)
             ON CONFLICT(project) DO UPDATE SET color = excluded.color",
            params![project, color],
        )
        .map(|_| ())
        .map_err(|e| format!("set project color: {e}"))
    }

    pub fn clear_project_color(&self, project: &str) -> rusqlite::Result<usize> {
        let conn = self.writer();
        conn.execute(
            "DELETE FROM project_color WHERE project = ?1",
            params![project.trim()],
        )
    }

    pub fn get_project_stacks(&self) -> rusqlite::Result<Vec<ProjectStack>> {
        let conn = self.read_conn();
        let mut stmt = conn.prepare_cached(
            "SELECT project, tag FROM project_stack ORDER BY project, ord, tag",
        )?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        let mut out: Vec<ProjectStack> = Vec::new();
        for (project, tag) in rows {
            if let Some(last) = out.last_mut().filter(|p| p.project == project) {
                if !last.stack.iter().any(|t| t.eq_ignore_ascii_case(&tag)) {
                    last.stack.push(tag);
                }
            } else {
                out.push(ProjectStack {
                    project,
                    stack: vec![tag],
                });
            }
        }
        Ok(out)
    }

    /// Replace a project's tech stack tags (preserving order, deduplicated
    /// case-insensitively). Passing an empty list clears the project's stack.
    pub fn set_project_stack(&self, project: &str, stack: &[String]) -> Result<(), String> {
        let project = project.trim();
        if project.is_empty() || project == "unknown" {
            return Err(format!("cannot tag project {project:?}"));
        }
        let mut cleaned: Vec<String> = Vec::new();
        for raw in stack {
            let tag = raw.trim();
            if tag.is_empty() {
                continue;
            }
            if !cleaned.iter().any(|t| t.eq_ignore_ascii_case(tag)) {
                cleaned.push(tag.to_string());
            }
        }
        let conn = self.writer();
        let canonical = project_canonical(&conn, project);
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("begin project stack transaction: {e}"))?;
        tx.execute(
            "DELETE FROM project_stack WHERE project = ?1",
            params![canonical],
        )
        .map_err(|e| format!("clear project stack: {e}"))?;
        for (ord, tag) in cleaned.iter().enumerate() {
            tx.execute(
                "INSERT INTO project_stack(project, tag, ord) VALUES(?1, ?2, ?3)",
                params![canonical, tag, ord as i64],
            )
            .map_err(|e| format!("insert project stack tag: {e}"))?;
        }
        tx.commit()
            .map_err(|e| format!("commit project stack: {e}"))?;
        Ok(())
    }

    /// The project a recorded folder counts under: its mapped target if one exists,
    /// else the folder itself. Flat by construction — writers never map a folder
    /// onto another mapped folder, so one join is always enough.
    pub fn project_canonical(&self, name: &str) -> String {
        let conn = self.read_conn();
        project_canonical(&conn, name)
    }

    /// Map every recorded folder to its project root. Idempotent and sticky.
    pub fn ensure_project_aliases(&self) -> rusqlite::Result<usize> {
        let conn = self.writer();
        let projects: Vec<String> = {
            let mut stmt = conn.prepare_cached(
                "SELECT DISTINCT project FROM usage_event WHERE project IS NOT NULL AND project != ''",
            )?;
            let rows = stmt.query_map([], |r| r.get(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        let mut n = 0;
        for p in &projects {
            n += fold_project_alias(&conn, p)?;
        }
        remap_project_colors(&conn)?;
        remap_project_stacks(&conn)?;
        Ok(n)
    }

    pub fn get_project_aliases(&self) -> rusqlite::Result<Vec<ProjectAlias>> {
        let conn = self.read_conn();
        let mut stmt = conn.prepare_cached(
            "SELECT alias, canonical FROM project_alias ORDER BY canonical, alias",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(ProjectAlias {
                    alias: r.get(0)?,
                    canonical: r.get(1)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Merge projects `names` into one project `canonical` (which must be one of the
    /// names). Folders already folded into an absorbed name follow it, so the table
    /// never forms alias -> alias chains. Idempotent.
    pub fn merge_projects(&self, names: &[String], canonical: &str) -> Result<usize, String> {
        if names.len() < 2 {
            return Err("merge needs at least two projects".to_string());
        }
        if canonical.is_empty() || canonical == "unknown" {
            return Err(format!("cannot merge into project {canonical:?}"));
        }
        if !names.iter().any(|n| n == canonical) {
            return Err("canonical project must be one of the merged names".to_string());
        }
        if names.iter().any(|n| n.is_empty() || n == "unknown") {
            return Err("the 'unknown' bucket cannot be merged".to_string());
        }
        let conn = self.writer();
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("begin merge transaction: {e}"))?;
        // A canonical kept separate from its own root must not keep that row.
        tx.execute(
            "DELETE FROM project_alias WHERE alias = ?1",
            params![canonical],
        )
        .map_err(|e| format!("clear canonical mapping: {e}"))?;
        let mut n = 0usize;
        let mut seen = std::collections::HashSet::new();
        for name in names.iter().filter(|n| *n != canonical) {
            if !seen.insert(name.clone()) {
                continue;
            }
            // Folders already folded into an absorbed name follow it to the new canonical.
            n += tx
                .execute(
                    "UPDATE project_alias SET canonical = ?1 WHERE canonical = ?2 AND alias <> ?1",
                    params![canonical, name],
                )
                .map_err(|e| format!("repoint folders: {e}"))?;
            n += tx
                .execute(
                    "INSERT INTO project_alias(alias, canonical) VALUES(?1, ?2)
                     ON CONFLICT(alias) DO UPDATE SET canonical = excluded.canonical",
                    params![name, canonical],
                )
                .map_err(|e| format!("set mapping: {e}"))?;
        }
        // Re-key pinned colors and tech stacks inside the transaction so a merge
        // moves folders and their annotations together or not at all.
        remap_project_colors(&tx).map_err(|e| format!("remap project colors: {e}"))?;
        remap_project_stacks(&tx).map_err(|e| format!("remap project stacks: {e}"))?;
        tx.commit().map_err(|e| format!("commit merge: {e}"))?;
        Ok(n)
    }

    /// Pin each folder as its own project — "no, keep this one separate". The row
    /// maps the folder to itself on purpose: a plain delete would hand it straight
    /// back to automatic root-folding on the next pass.
    pub fn unmerge_projects(&self, names: &[String]) -> Result<usize, String> {
        let conn = self.writer();
        let mut n = 0usize;
        let mut seen = std::collections::HashSet::new();
        for name in names.iter().filter(|n| !n.is_empty() && **n != "unknown") {
            if !seen.insert(name.clone()) {
                continue;
            }
            n += conn
                .execute(
                    "INSERT INTO project_alias(alias, canonical) VALUES(?1, ?1)
                     ON CONFLICT(alias) DO UPDATE SET canonical = excluded.canonical",
                    params![name],
                )
                .map_err(|e| format!("keep project separate: {e}"))?;
        }
        Ok(n)
    }

    /// Hand each folder back to automatic root-folding: drop the recorded decision
    /// and resolve it again from the filesystem.
    pub fn regroup_projects(&self, names: &[String]) -> Result<usize, String> {
        let conn = self.writer();
        let mut n = 0usize;
        let mut seen = std::collections::HashSet::new();
        for name in names.iter().filter(|n| !n.is_empty() && **n != "unknown") {
            if !seen.insert(name.clone()) {
                continue;
            }
            conn.execute("DELETE FROM project_alias WHERE alias = ?1", params![name])
                .map_err(|e| format!("clear project mapping: {e}"))?;
            n += fold_project_alias(&conn, name)
                .map_err(|e| format!("resolve project root: {e}"))?;
        }
        Ok(n)
    }
}

/// The project a recorded folder counts under: its mapped target if one exists,
/// else the folder itself.
fn project_canonical(conn: &Connection, name: &str) -> String {
    conn.query_row(
        "SELECT canonical FROM project_alias WHERE alias = ?1",
        params![name],
        |r| r.get::<_, String>(0),
    )
    .unwrap_or_else(|_| name.to_string())
}

/// Fold `folder` into its project root, unless a mapping already exists —
/// a decision once recorded (by hand or by an earlier pass) is never recomputed,
/// so history stays grouped the way it was seen and repos that moved stay whole.
/// Runs on the caller's connection so ingest can fold inside its own transaction.
fn fold_project_alias(conn: &Connection, folder: &str) -> rusqlite::Result<usize> {
    let root = resolve_project_root(folder);
    if root == folder {
        return Ok(0);
    }
    // Follow the root's own mapping so the table can never form a chain.
    let canonical = project_canonical(conn, &root);
    conn.execute(
        "INSERT INTO project_alias(alias, canonical) VALUES(?1, ?2) ON CONFLICT(alias) DO NOTHING",
        params![folder, canonical],
    )
}

/// Re-key pinned colors through the mapping table, so a color pinned on a folder
/// follows it into the project it counts under. `OR IGNORE` keeps one pin when
/// two pinned folders land on the same project.
fn remap_project_colors(conn: &Connection) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE OR IGNORE project_color
         SET project = (SELECT canonical FROM project_alias WHERE alias = project_color.project)
         WHERE project IN (SELECT alias FROM project_alias)",
        [],
    )
}

fn remap_project_stacks(conn: &Connection) -> rusqlite::Result<usize> {
    let n = conn.execute(
        "UPDATE OR IGNORE project_stack
         SET project = (SELECT canonical FROM project_alias WHERE alias = project_stack.project)
         WHERE project IN (SELECT alias FROM project_alias WHERE alias != canonical)",
        [],
    )?;
    conn.execute(
        "DELETE FROM project_stack
         WHERE project IN (SELECT alias FROM project_alias WHERE alias != canonical)",
        [],
    )?;
    Ok(n)
}

/// `#RRGGBB` → `#rrggbb`; anything else (short hex, names, rgba) → None.
fn normalize_hex(color: &str) -> Option<String> {
    let hex = color.trim().strip_prefix('#')?;
    (hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| format!("#{}", hex.to_ascii_lowercase()))
}

/// Fold a recorded working directory into its project: the nearest enclosing
/// directory that holds a `.git`. Subfolders of one repository — however deep,
/// `node_modules/.pnpm/.../dist/core` included — are one project, while nested
/// repositories (submodules, vendored checkouts) stay their own.
///
/// Strings that are not POSIX paths (WackChatter's synthetic "WackChatter ·
/// <character>" names) and folders outside any repository come back unchanged.
/// Missing directories are walked through rather than rejected, so a deleted deep
/// folder still lands on the repository above it.
pub fn resolve_project_root(path: &str) -> String {
    if path.is_empty() || !path.starts_with('/') {
        return path.to_string();
    }
    let base = path.trim_end_matches('/');
    let start = if base.is_empty() { "/" } else { base };
    let mut dir = Path::new(start);
    loop {
        if dir.join(".git").exists() {
            return dir.to_string_lossy().into_owned();
        }
        match dir.parent() {
            Some(parent) => dir = parent,
            None => return path.to_string(),
        }
    }
}

/// A checked-out read connection, returned to the pool on drop.
pub struct PooledRead<'a> {
    conn: Option<Connection>,
    pool: &'a Mutex<Vec<Connection>>,
}

impl Drop for PooledRead<'_> {
    fn drop(&mut self) {
        if let Some(conn) = self.conn.take() {
            let mut pool = self.pool.lock().unwrap_or_else(|p| p.into_inner());
            if pool.len() < MAX_POOLED_READERS {
                pool.push(conn);
            }
        }
    }
}

impl std::ops::Deref for PooledRead<'_> {
    type Target = Connection;

    fn deref(&self) -> &Connection {
        self.conn.as_ref().expect("connection present until drop")
    }
}

/// A connection to run read-only queries against. Deref's to `Connection`.
pub enum ReadConn<'a> {
    /// A pooled (or freshly opened) dedicated reader — file-backed stores.
    Pooled(PooledRead<'a>),
    /// The writer connection itself: in-memory stores have no second
    /// connection to hand out, and a failed reader open degrades to this.
    Writer(MutexGuard<'a, Connection>),
}

impl std::ops::Deref for ReadConn<'_> {
    type Target = Connection;

    fn deref(&self) -> &Connection {
        match self {
            ReadConn::Pooled(c) => c,
            ReadConn::Writer(c) => c,
        }
    }
}

/// Open an additional read connection to TokenTrail's own store. WAL mode is a
/// persistent property of the database file, so this reader never blocks the
/// writer and vice versa. Plain read-write flags on purpose: a read-only
/// connection cannot create a WAL's shared-memory index, and the `immutable=1`
/// fallback `open_readonly` uses for harness files would ignore the WAL and
/// read stale data.
fn open_reader(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(conn)
}

/// Open a harness-owned SQLite database strictly read-only.
pub fn open_readonly(path: &Path) -> rusqlite::Result<Connection> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
        | OpenFlags::SQLITE_OPEN_NO_MUTEX
        | OpenFlags::SQLITE_OPEN_URI;
    match Connection::open_with_flags(path, flags) {
        Ok(c) => Ok(c),
        Err(_) => {
            Connection::open_with_flags(format!("file:{}?immutable=1", path.display()), flags)
        }
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
        assert_eq!(
            aliases[0],
            ModelAlias {
                alias: "glm-5.3".into(),
                canonical: "GLM-5.3".into()
            }
        );

        store.remove_aliases_for("GLM-5.3").unwrap();
        assert!(store.get_model_aliases().unwrap().is_empty());
    }

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("tokentrail-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn ev(id: &str, project: Option<&str>) -> UsageEvent {
        UsageEvent {
            source: Source::Zcode,
            source_event_id: id.to_string(),
            ts: 1_700_000_000_000,
            session_id: None,
            project: project.map(String::from),
            provider: None,
            provider_name: None,
            model: Some("gpt-5".to_string()),
            input_tokens: 1,
            output_tokens: 1,
            reasoning_tokens: None,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            duration_ms: None,
            ttft_ms: None,
            is_subagent: false,
            estimated: false,
            purpose: None,
            outcome: None,
            workspace: None,
            subagent_id: None,
            mode: None,
        }
    }

    #[test]
    fn resolve_project_root_folds_subfolders_to_nearest_git_root() {
        let repo = tmp_dir("root");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let deep = repo.join("node_modules/pkg/dist/core");
        std::fs::create_dir_all(&deep).unwrap();
        assert_eq!(
            resolve_project_root(&deep.to_string_lossy()),
            repo.to_string_lossy()
        );

        // a nested repository stays its own project
        let inner = repo.join("vendor/inner");
        std::fs::create_dir_all(inner.join(".git")).unwrap();
        std::fs::create_dir_all(inner.join("src")).unwrap();
        assert_eq!(
            resolve_project_root(&inner.join("src").to_string_lossy()),
            inner.to_string_lossy()
        );

        // deleted deep folders still land on the repository above them
        assert_eq!(
            resolve_project_root(&repo.join("gone/long/ago").to_string_lossy()),
            repo.to_string_lossy()
        );

        // outside any repository: unchanged
        let loose = tmp_dir("loose");
        let scripts = loose.join("notes/scripts");
        std::fs::create_dir_all(&scripts).unwrap();
        assert_eq!(
            resolve_project_root(&scripts.to_string_lossy()),
            scripts.to_string_lossy()
        );

        // synthetic non-path names are never rewritten
        assert_eq!(
            resolve_project_root("WackChatter · Lyra"),
            "WackChatter · Lyra"
        );
    }

    #[test]
    fn project_folders_fold_and_decisions_stay_sticky() {
        let store = test_store();
        let repo = tmp_dir("sticky");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let sub = repo.join("worker/src");
        std::fs::create_dir_all(&sub).unwrap();
        let sub_s = sub.to_string_lossy().to_string();
        let repo_s = repo.to_string_lossy().to_string();

        store.insert_events(&[ev("1", Some(&sub_s))]).unwrap();
        // ingest folds the subfolder into its repository immediately
        assert_eq!(store.project_canonical(&sub_s), repo_s);

        // a decision made by hand is never recomputed away
        store.unmerge_projects(&[sub_s.clone()]).unwrap();
        store.ensure_project_aliases().unwrap();
        store.insert_events(&[ev("2", Some(&sub_s))]).unwrap();
        assert_eq!(store.project_canonical(&sub_s), sub_s);

        // handing it back to automatic folding restores the repository grouping
        store.regroup_projects(&[sub_s.clone()]).unwrap();
        assert_eq!(store.project_canonical(&sub_s), repo_s);
    }

    #[test]
    fn merged_projects_stay_flat() {
        let store = test_store();
        store
            .merge_projects(&["/a".into(), "/b".into()], "/a")
            .unwrap();
        assert_eq!(store.project_canonical("/b"), "/a");

        // absorbing a project carries its folded folders along, no chains
        store
            .merge_projects(&["/a".into(), "/c".into()], "/c")
            .unwrap();
        assert_eq!(store.project_canonical("/a"), "/c");
        assert_eq!(store.project_canonical("/b"), "/c");

        // the unknown bucket is not a project and cannot be merged
        assert!(store
            .merge_projects(&["/c".into(), "unknown".into()], "/c")
            .is_err());

        store.unmerge_projects(&["/b".into()]).unwrap();
        assert_eq!(store.project_canonical("/b"), "/b");
    }

    #[test]
    fn project_colors_follow_merged_folders() {
        let store = test_store();
        store.set_project_color("/repo/sub", "#FF0000").unwrap();
        store
            .merge_projects(&["/repo/sub".into(), "/repo".into()], "/repo")
            .unwrap();
        assert_eq!(
            store.get_project_colors().unwrap(),
            vec![ProjectColor {
                project: "/repo".into(),
                color: "#ff0000".into()
            }]
        );
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
        store
            .merge_models(&["A".into(), "B".into(), "C".into()], "A")
            .unwrap();
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
        assert_eq!(
            store.get_hidden_models().unwrap(),
            vec!["codex-auto-review"]
        );

        // Re-hiding is a no-op, and multiple names are kept sorted.
        store
            .hide_models(&["codex-auto-review".into(), "gpt-5".into()])
            .unwrap();
        assert_eq!(
            store.get_hidden_models().unwrap(),
            vec!["codex-auto-review", "gpt-5"]
        );

        store.unhide_model("codex-auto-review").unwrap();
        assert_eq!(store.get_hidden_models().unwrap(), vec!["gpt-5"]);
    }

    #[test]
    fn project_color_round_trip() {
        let store = test_store();
        assert!(store.get_project_colors().unwrap().is_empty());

        store.set_project_color("/code/beta", "#FF4D00").unwrap();
        store.set_project_color("/code/alpha", "#00c2c2").unwrap();
        // stored lowercase, listed by project
        assert_eq!(
            store.get_project_colors().unwrap(),
            vec![
                ProjectColor {
                    project: "/code/alpha".into(),
                    color: "#00c2c2".into()
                },
                ProjectColor {
                    project: "/code/beta".into(),
                    color: "#ff4d00".into()
                },
            ]
        );

        // setting again overwrites rather than adding a second row
        store.set_project_color("/code/beta", "#7c5cff").unwrap();
        let colors = store.get_project_colors().unwrap();
        assert_eq!(colors.len(), 2);
        assert_eq!(colors[1].color, "#7c5cff");

        store.clear_project_color("/code/alpha").unwrap();
        assert_eq!(
            store.get_project_colors().unwrap(),
            vec![ProjectColor {
                project: "/code/beta".into(),
                color: "#7c5cff".into()
            }]
        );
    }

    #[test]
    fn project_color_validates_input() {
        let store = test_store();
        for bad in [
            "",
            "ff4d00",
            "#f40",
            "#ff4d0",
            "#ff4d00aa",
            "#gg4d00",
            "red",
            "rgba(0,0,0,1)",
        ] {
            assert!(
                store.set_project_color("/code/a", bad).is_err(),
                "accepted {bad:?}"
            );
        }
        assert!(store.set_project_color("unknown", "#ff4d00").is_err());
        assert!(store.set_project_color("  ", "#ff4d00").is_err());
        assert!(store.get_project_colors().unwrap().is_empty());
    }

    #[test]
    fn project_stack_round_trip() {
        let store = test_store();
        assert!(store.get_project_stacks().unwrap().is_empty());

        store
            .set_project_stack(
                "/code/beta",
                &[
                    "Tauri".into(),
                    "SvelteKit".into(),
                    "tauri".into(),
                    "  ".into(),
                    "Rust".into(),
                ],
            )
            .unwrap();
        store
            .set_project_stack("/code/alpha", &["Electron".into(), "React".into()])
            .unwrap();

        assert_eq!(
            store.get_project_stacks().unwrap(),
            vec![
                ProjectStack {
                    project: "/code/alpha".into(),
                    stack: vec!["Electron".into(), "React".into()],
                },
                ProjectStack {
                    project: "/code/beta".into(),
                    stack: vec!["Tauri".into(), "SvelteKit".into(), "Rust".into()],
                },
            ]
        );

        // Updating replaces the list in the new order
        store
            .set_project_stack("/code/beta", &["Rust".into(), "SQLite".into()])
            .unwrap();
        assert_eq!(
            store.get_project_stacks().unwrap()[1].stack,
            vec!["Rust".to_string(), "SQLite".to_string()]
        );

        // Empty list clears the project's stack
        store.set_project_stack("/code/alpha", &[]).unwrap();
        assert_eq!(
            store.get_project_stacks().unwrap(),
            vec![ProjectStack {
                project: "/code/beta".into(),
                stack: vec!["Rust".into(), "SQLite".into()],
            }]
        );
    }

    #[test]
    fn project_stack_validates_input() {
        let store = test_store();
        assert!(store
            .set_project_stack("unknown", &["Tauri".into()])
            .is_err());
        assert!(store.set_project_stack("   ", &["Tauri".into()]).is_err());
        assert!(store.get_project_stacks().unwrap().is_empty());
    }

    #[test]
    fn project_stacks_follow_merged_folders() {
        let store = test_store();
        store
            .set_project_stack("/repo/sub", &["Rust".into(), "Tauri".into()])
            .unwrap();
        store
            .set_project_stack("/repo", &["SvelteKit".into(), "Rust".into()])
            .unwrap();
        store
            .merge_projects(&["/repo/sub".into(), "/repo".into()], "/repo")
            .unwrap();
        assert_eq!(
            store.get_project_stacks().unwrap(),
            vec![ProjectStack {
                project: "/repo".into(),
                stack: vec!["SvelteKit".into(), "Rust".into(), "Tauri".into()],
            }]
        );
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
            provider_name: None,
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
            purpose: None,
            outcome: None,
            workspace: None,
            subagent_id: None,
            mode: None,
        };
        store.insert_events(&[e]).unwrap();
        // Insert priced it at $18 (1M in + 1M out at $3/$15).
        let before: f64 = store
            .read_conn()
            .query_row("SELECT cost_usd FROM usage_event", [], |r| r.get(0))
            .unwrap();
        assert!((before - 18.0).abs() < 1e-9);

        // Sabotage the stored cost, then reprice_all restores it.
        store
            .read_conn()
            .execute("UPDATE usage_event SET cost_usd = 1.0", [])
            .unwrap();
        assert_eq!(store.reprice_all().unwrap(), 1);
        let after: f64 = store
            .read_conn()
            .query_row("SELECT cost_usd FROM usage_event", [], |r| r.get(0))
            .unwrap();
        assert!((after - 18.0).abs() < 1e-9);
    }

    /// The reader pool and the writer are separate connections, so reads must
    /// run while batches commit — never failing with SQLITE_BUSY or starving.
    #[test]
    fn reads_run_concurrently_with_writes() {
        use std::sync::Arc;
        use std::time::Instant;

        let store = Arc::new(Store::open(&tmp_dir("concurrent").join("usage.db")).unwrap());
        let batch: Vec<UsageEvent> = (0..200).map(|i| ev(&format!("w{i}"), None)).collect();

        let reader_store = store.clone();
        let writer = std::thread::spawn(move || {
            for chunk in batch.chunks(50) {
                reader_store.insert_events(chunk).unwrap();
            }
        });

        let deadline = Instant::now() + std::time::Duration::from_secs(10);
        let mut reads = 0;
        while !writer.is_finished() {
            let conn = store.read_conn();
            let n: i64 = conn
                .query_row("SELECT COUNT(*) FROM usage_event", [], |r| r.get(0))
                .unwrap();
            let _ = n;
            reads += 1;
            assert!(Instant::now() < deadline, "reads starved while writes ran");
        }
        writer.join().unwrap();
        assert!(reads > 0, "no read completed during the writes");

        let conn = store.read_conn();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM usage_event", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n as usize, 200);
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
            provider_name: None,
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
            purpose: None,
            outcome: None,
            workspace: None,
            subagent_id: None,
            mode: None,
        };
        let e2 = UsageEvent {
            source: Source::Zcode,
            source_event_id: "r2".into(),
            ts: 2000,
            session_id: None,
            project: None,
            provider: None,
            provider_name: None,
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
            purpose: None,
            outcome: None,
            workspace: None,
            subagent_id: None,
            mode: None,
        };
        let e3 = UsageEvent {
            source: Source::Zcode,
            source_event_id: "r3".into(),
            ts: 3000,
            session_id: None,
            project: None,
            provider: None,
            provider_name: None,
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
            purpose: None,
            outcome: None,
            workspace: None,
            subagent_id: None,
            mode: None,
        };
        let e4 = UsageEvent {
            source: Source::Zcode,
            source_event_id: "r4".into(),
            ts: 4000,
            session_id: None,
            project: None,
            provider: None,
            provider_name: None,
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
            purpose: None,
            outcome: None,
            workspace: None,
            subagent_id: None,
            mode: None,
        };
        store.insert_events(&[e1, e2, e3, e4]).unwrap();

        let raw = store.get_raw_models().unwrap();
        assert_eq!(raw, vec!["gpt-4o", "gpt-4o-2024-08-06"]);
    }

    #[test]
    fn schema_creates_expected_indexes() {
        let store = test_store();
        let conn = store.read_conn();
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='index' AND tbl_name='usage_event' ORDER BY name")
            .unwrap();
        let names: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert!(names.contains(&"idx_usage_ts".to_string()));
        assert!(names.contains(&"idx_usage_source_ts".to_string()));
        assert!(names.contains(&"idx_usage_session".to_string()));
        assert!(names.contains(&"idx_usage_model".to_string()));
        assert!(names.contains(&"idx_usage_project".to_string()));

        let session_sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='index' AND name='idx_usage_session'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(session_sql.to_lowercase().contains("ts desc"));
    }

    #[test]
    fn open_migrates_legacy_indexes() {
        let dir = tmp_dir("legacy_idx");
        let db_path = dir.join("usage.db");

        // Manually create legacy schema with 2-column idx_usage_session and without model/project indexes
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE usage_event (
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
                CREATE INDEX idx_usage_ts ON usage_event(ts);
                CREATE INDEX idx_usage_source_ts ON usage_event(source, ts);
                CREATE INDEX idx_usage_session ON usage_event(source, session_id);",
            )
            .unwrap();
        }

        // Open with Store::open, which should run migrations
        let store = Store::open(&db_path).unwrap();
        let conn = store.read_conn();

        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='index' AND tbl_name='usage_event' ORDER BY name")
            .unwrap();
        let names: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert!(names.contains(&"idx_usage_model".to_string()));
        assert!(names.contains(&"idx_usage_project".to_string()));
        assert!(names.contains(&"idx_usage_session".to_string()));

        let session_sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='index' AND name='idx_usage_session'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(session_sql.to_lowercase().contains("ts desc"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
