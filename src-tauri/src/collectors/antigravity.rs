use crate::models::{Source, UsageEvent};
use crate::store::{open_readonly, Store};
use std::collections::HashMap;
use std::path::Path;
use std::time::UNIX_EPOCH;

/// Google Antigravity stores one SQLite DB per conversation under
/// ~/.gemini/antigravity/conversations/<uuid>.db. LLM calls live in the
/// `gen_metadata` table as protobuf blobs (the Windsurf-lineage
/// ChatModelMetadata message). The wire layout was mapped against local data:
///   field 1 (message) ->
///     4:  ModelUsageStats  { 2: input, 3: output_total, 5: cache_read,
///                            9: thinking, 11: request_id }
///     9:  timing          { 4: start Timestamp{1: s, 2: ns}, 8: elapsed Duration }
///     19: model id string (e.g. "gemini-3.7-flash")
/// `output_total` is thinking + response_output; per repo convention reasoning
/// is kept out of output_tokens. Rows are append-only with a contiguous `idx`,
/// so a per-file watermark on idx keeps re-ingestion cheap and idempotent.
///
/// Antigravity builds from Aug 2026 dropped the absolute timestamp (timing
/// field 4) and now record only internal counters there. Wall-clock times
/// moved to the `steps` table: each step's metadata blob carries start/end
/// Timestamps (fields 1/32) and the LLM call info (field 9) whose request id
/// matches the gen row's usage message (field 11), so new-format rows are
/// joined to their step for ts and duration. Rows whose join fails fall back
/// to the conversation file's mtime instead of being dropped.
/// Sub-trajectories (agent tasks) currently surface as their own conversation
/// DBs; there is no in-file subagent marker to read.
///
/// Bump FORMAT_VERSION whenever parsing above changes shape: rows the previous
/// parser skipped still advanced their watermark, so a one-time reset forces a
/// full re-read (events upsert on (source, source_event_id), making it safe).
const FORMAT_VERSION: i64 = 2;

pub fn collect(store: &Store, home: &Path) -> Result<usize, String> {
    let root = home.join(".gemini/antigravity/conversations");
    if !root.exists() {
        return Ok(0);
    }
    if store.get_watermark("antigravity_format") < FORMAT_VERSION {
        store.clear_watermarks("antigravity:%");
        store.set_watermark("antigravity_format", FORMAT_VERSION);
    }
    let files = sorted_glob_db(&root)?;
    let mut processed = 0usize;
    for path in files {
        // a crashed or mid-migration conversation DB must not stall the others
        let Ok(conn) = open_readonly(&path) else { continue };
        let Some(uuid) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        let wm_key = format!("antigravity:{uuid}");
        let from = store.get_watermark(&wm_key);
        let project = read_project(&conn);
        let step_times = read_step_times(&conn);
        let mtime_ms = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let rows = conn
            .prepare("SELECT idx, data FROM gen_metadata WHERE idx >= ?1 ORDER BY idx")
            .and_then(|mut s| {
                s.query_map([from], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?))
                })
                .map(|rows| rows.filter_map(|row| row.ok()).collect::<Vec<_>>())
            });
        let Ok(rows) = rows else { continue };
        if rows.is_empty() {
            continue;
        }
        let mut events = Vec::new();
        let mut max_idx = from;
        for (idx, blob) in rows {
            max_idx = max_idx.max(idx);
            let Some(g) = parse_gen(&blob) else { continue };
            if g.input + g.output_total + g.cache_read == 0 {
                continue;
            }
            let step = g
                .request_id
                .as_deref()
                .and_then(|r| step_times.get(r))
                .copied();
            // legacy blobs carry their own timestamp; new-format rows borrow
            // their step's start, and only fall back to mtime if the join fails
            let ts = if g.ts > 0 {
                g.ts
            } else {
                step.filter(|(s, _)| *s > 0)
                    .map(|(s, _)| s)
                    .unwrap_or(mtime_ms)
            };
            if ts <= 0 {
                // no timestamp anywhere we know to look: leave the row for a
                // future parser rather than store an event dated 1970
                continue;
            }
            let duration_ms = g
                .duration_ms
                .or_else(|| step.and_then(|(s, e)| (e > s).then_some(e - s)));
            let thinking = g.thinking.min(g.output_total);
            events.push(UsageEvent {
                source: Source::Antigravity,
                source_event_id: format!("{uuid}:{idx}"),
                ts,
                session_id: Some(uuid.to_string()),
                project: project.clone(),
                provider: provider_for(g.model.as_deref()),
                model: g.model,
                input_tokens: g.input,
                output_tokens: g.output_total - thinking,
                reasoning_tokens: (thinking > 0).then_some(thinking),
                cache_read_tokens: g.cache_read,
                cache_write_tokens: 0,
                duration_ms,
                ttft_ms: None,
                is_subagent: false,
                estimated: false,
            });
        }
        processed += store
            .insert_events(&events)
            .map_err(|e| format!("antigravity insert {uuid}: {e}"))?;
        store.set_watermark(&wm_key, max_idx + 1);
    }
    Ok(processed)
}

struct GenCall {
    ts: i64,
    input: i64,
    output_total: i64,
    thinking: i64,
    cache_read: i64,
    duration_ms: Option<i64>,
    model: Option<String>,
    request_id: Option<String>,
}

/// Workspace URI from the trajectory metadata blob: field 1 -> field 1.
fn read_project(conn: &rusqlite::Connection) -> Option<String> {
    let blob: Vec<u8> = conn
        .query_row(
            "SELECT data FROM trajectory_metadata_blob WHERE id = 'main'",
            [],
            |r| r.get(0),
        )
        .ok()?;
    let workspace = Pb::new(&blob).fields().find(|(f, wt, _)| *f == 1 && *wt == 2)?.2;
    let inner = Pb::new(workspace.bytes()?);
    let uri = inner
        .fields()
        .find(|(f, wt, _)| *f == 1 && *wt == 2)?
        .2
        .bytes()?;
    let s = String::from_utf8_lossy(uri).trim_start_matches("file://").to_string();
    (!s.is_empty()).then_some(s)
}

fn provider_for(model: Option<&str>) -> Option<String> {
    let m = model?;
    if m.starts_with("claude") {
        Some("anthropic".into())
    } else if m.starts_with("gpt") {
        Some("openai".into())
    } else {
        Some("google".into())
    }
}

/// request_id -> (start_ms, end_ms) from the `steps` table. New-format
/// conversations keep wall-clock times here: each step's metadata blob has a
/// start/end Timestamp (fields 1/32) and LLM call info (repeated field 9)
/// whose request id matches the gen row's usage message. A missing `steps`
/// table or unparseable blobs yield an empty map and the caller falls back.
fn read_step_times(conn: &rusqlite::Connection) -> HashMap<String, (i64, i64)> {
    let mut map = HashMap::new();
    let Ok(mut stmt) = conn.prepare("SELECT metadata FROM steps ORDER BY idx") else {
        return map;
    };
    let Ok(rows) = stmt.query_map([], |r| r.get::<_, Vec<u8>>(0)) else {
        return map;
    };
    for blob in rows.flatten() {
        let mut start = 0i64;
        let mut end = 0i64;
        let mut call_ids = Vec::new();
        for (f, wt, v) in Pb::new(&blob).fields() {
            match (f, wt) {
                (1, 2) => {
                    if let Some(b) = v.bytes() {
                        start = timestamp_ms(b);
                    }
                }
                (9, 2) => {
                    if let Some(id) = v.bytes().and_then(|g| field_str(g, 11)) {
                        call_ids.push(id);
                    }
                }
                (32, 2) => {
                    if let Some(b) = v.bytes() {
                        end = timestamp_ms(b);
                    }
                }
                _ => {}
            }
        }
        for id in call_ids {
            map.insert(id, (start, end));
        }
    }
    map
}

fn field_str(buf: &[u8], num: u32) -> Option<String> {
    let v = Pb::new(buf).fields().find(|(f, wt, _)| *f == num && *wt == 2)?.2;
    Some(String::from_utf8_lossy(v.bytes()?).into_owned())
}

fn parse_gen(blob: &[u8]) -> Option<GenCall> {
    let mut g = GenCall {
        ts: 0,
        input: 0,
        output_total: 0,
        thinking: 0,
        cache_read: 0,
        duration_ms: None,
        model: None,
        request_id: None,
    };
    // the payload message is field 1; take the last occurrence (proto semantics)
    let mut payload: Option<&[u8]> = None;
    for (f, wt, v) in Pb::new(blob).fields() {
        if f == 1 && wt == 2 {
            payload = v.bytes();
        }
    }
    let payload = payload?;
    for (f, wt, v) in Pb::new(payload).fields() {
        match (f, wt) {
            (4, 2) => {
                for (uf, _, uv) in Pb::new(v.bytes()?).fields() {
                    match uf {
                        2 => g.input = uv.varint().as_i64(),
                        3 => g.output_total = uv.varint().as_i64(),
                        5 => g.cache_read = uv.varint().as_i64(),
                        9 => g.thinking = uv.varint().as_i64(),
                        11 => {
                            if let Some(s) = uv.bytes() {
                                g.request_id = Some(String::from_utf8_lossy(s).into_owned());
                            }
                        }
                        _ => {}
                    }
                }
            }
            (9, 2) => {
                for (tf, _, tv) in Pb::new(v.bytes()?).fields() {
                    match tf {
                        4 => g.ts = timestamp_ms(tv.bytes()?),
                        8 => g.duration_ms = Some(duration_ms(tv.bytes()?)),
                        _ => {}
                    }
                }
            }
            (19, 2) => {
                let m = String::from_utf8_lossy(v.bytes()?).to_string();
                if !m.is_empty() {
                    g.model = Some(m);
                }
            }
            _ => {}
        }
    }
    // ts may legitimately be 0 (new-format rows carry no timestamp); the
    // caller resolves it via the steps join or the file mtime
    Some(g)
}

fn timestamp_ms(blob: &[u8]) -> i64 {
    let mut sec = 0u64;
    let mut nanos = 0u32;
    for (f, _, v) in Pb::new(blob).fields() {
        match f {
            1 => sec = v.varint().as_u64(),
            2 => nanos = v.varint().as_u64().min(u32::MAX as u64) as u32,
            _ => {}
        }
    }
    let sec = sec.min(i64::MAX as u64) as i64;
    sec.saturating_mul(1000).saturating_add((nanos / 1_000_000) as i64)
}

fn duration_ms(blob: &[u8]) -> i64 {
    let mut sec = 0u64;
    let mut nanos = 0u32;
    for (f, _, v) in Pb::new(blob).fields() {
        match f {
            1 => sec = v.varint().as_u64(),
            2 => nanos = v.varint().as_u64().min(u32::MAX as u64) as u32,
            _ => {}
        }
    }
    let sec = sec.min(i64::MAX as u64) as i64;
    sec.saturating_mul(1000).saturating_add((nanos / 1_000_000) as i64)
}

/// Minimal protobuf wire reader: walks varint-tagged fields and skips
/// everything we don't ask for by number, so unknown or reordered fields in
/// future Antigravity builds cannot break parsing of the ones we track.
struct Pb<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Pb<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Pb { buf, pos: 0 }
    }

    fn fields(&self) -> FieldIter<'a> {
        FieldIter { pb: Pb { buf: self.buf, pos: self.pos } }
    }
}

struct FieldIter<'a> {
    pb: Pb<'a>,
}

impl<'a> Iterator for FieldIter<'a> {
    type Item = (u32, u8, Value<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        if self.pb.pos >= self.pb.buf.len() {
            return None;
        }
        let key = varint(self.pb.buf, &mut self.pb.pos)?;
        let field = (key >> 3) as u32;
        let wt = (key & 7) as u8;
        let value = match wt {
            0 => Value::Varint(varint(self.pb.buf, &mut self.pb.pos)?),
            1 => {
                let s = self.pb.pos;
                self.pb.pos = s.checked_add(8)?;
                self.pb.buf.get(s..s + 8)?;
                Value::Fixed
            }
            2 => {
                let len = varint(self.pb.buf, &mut self.pb.pos)? as usize;
                let s = self.pb.pos;
                let e = s.checked_add(len)?;
                self.pb.pos = e;
                Value::Bytes(self.pb.buf.get(s..e)?)
            }
            5 => {
                let s = self.pb.pos;
                self.pb.pos = s.checked_add(4)?;
                self.pb.buf.get(s..s + 4)?;
                Value::Fixed
            }
            _ => return None,
        };
        Some((field, wt, value))
    }
}

enum Value<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
    /// fixed-width payloads we never inspect; carried only so the iterator
    /// can advance past them
    Fixed,
}

impl<'a> Value<'a> {
    // tied to the underlying buffer, not the borrowed `self`, so parsed
    // sub-slices outlive the loop variable that produced them
    fn bytes(&self) -> Option<&'a [u8]> {
        match self {
            Value::Bytes(b) => Some(b),
            _ => None,
        }
    }

    fn varint(&self) -> VarintVal {
        match self {
            Value::Varint(v) => VarintVal(*v),
            _ => VarintVal(0),
        }
    }
}

#[derive(Clone, Copy)]
struct VarintVal(u64);

impl VarintVal {
    fn as_u64(self) -> u64 {
        self.0
    }

    fn as_i64(self) -> i64 {
        self.0.min(i64::MAX as u64) as i64
    }
}

fn varint(buf: &[u8], pos: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    let mut shift = 0u32;
    loop {
        let b = *buf.get(*pos)?;
        *pos += 1;
        v |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return Some(v);
        }
        shift = shift.checked_add(7)?;
        if shift >= 64 {
            return None;
        }
    }
}

fn sorted_glob_db(root: &Path) -> Result<Vec<std::path::PathBuf>, String> {
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(root)
        .map_err(|e| format!("antigravity list {}: {e}", root.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "db"))
        .collect();
    files.sort();
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collectors::{test_home, test_store};

    // --- tiny protobuf writer for fixtures ---

    fn tag(field: u32, wt: u8) -> Vec<u8> {
        let mut v = vec![];
        write_varint(&mut v, ((field as u64) << 3) | wt as u64);
        v
    }

    fn write_varint(out: &mut Vec<u8>, mut v: u64) {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                return;
            }
            out.push(b | 0x80);
        }
    }

    fn bytes_field(out: &mut Vec<u8>, field: u32, payload: &[u8]) {
        out.extend(tag(field, 2));
        write_varint(out, payload.len() as u64);
        out.extend_from_slice(payload);
    }

    fn varint_field(out: &mut Vec<u8>, field: u32, v: u64) {
        out.extend(tag(field, 0));
        write_varint(out, v);
    }

    fn ts_msg(sec: u64, nanos: u64) -> Vec<u8> {
        let mut m = vec![];
        varint_field(&mut m, 1, sec);
        varint_field(&mut m, 2, nanos);
        m
    }

    fn usage_msg(input: u64, output: u64, cache: u64, thinking: u64) -> Vec<u8> {
        let mut m = vec![];
        varint_field(&mut m, 2, input);
        varint_field(&mut m, 3, output);
        if cache > 0 {
            varint_field(&mut m, 5, cache);
        }
        if thinking > 0 {
            varint_field(&mut m, 9, thinking);
            varint_field(&mut m, 10, output - thinking);
        }
        m
    }

    fn gen_blob(sec: u64, input: u64, output: u64, cache: u64, thinking: u64, model: &str) -> Vec<u8> {
        let mut timing = vec![];
        bytes_field(&mut timing, 4, &ts_msg(sec, 250_000_000));
        let mut payload = vec![];
        bytes_field(&mut payload, 4, &usage_msg(input, output, cache, thinking));
        bytes_field(&mut payload, 9, &timing);
        bytes_field(&mut payload, 19, model.as_bytes());
        let mut blob = vec![];
        bytes_field(&mut blob, 1, &payload);
        blob
    }

    fn traj_blob(workspace: &str) -> Vec<u8> {
        let mut ws = vec![];
        bytes_field(&mut ws, 1, format!("file://{workspace}").as_bytes());
        let mut blob = vec![];
        bytes_field(&mut blob, 1, &ws);
        blob
    }

    fn seed(home: &Path) {
        let dir = home.join(".gemini/antigravity/conversations");
        std::fs::create_dir_all(&dir).unwrap();
        let conn = rusqlite::Connection::open(dir.join("11111111-2222-3333-4444-555555555555.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE gen_metadata (idx integer, data blob, size integer NOT NULL DEFAULT 0, PRIMARY KEY (idx));
             CREATE TABLE trajectory_metadata_blob (id text DEFAULT 'main', data blob, PRIMARY KEY (id));",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO trajectory_metadata_blob (id, data) VALUES ('main', ?1)",
            rusqlite::params![traj_blob("/Users/x/dev/proj")],
        )
        .unwrap();
        for (idx, blob) in [
            (0, gen_blob(1_700_000_000, 1000, 300, 5000, 100, "gemini-3.7-flash")),
            (1, gen_blob(1_700_000_060, 0, 0, 0, 0, "gemini-3.7-flash")), // no tokens: skipped
            (2, gen_blob(1_700_000_120, 3000, 400, 6000, 0, "gemini-pro-default")),
        ] {
            conn.execute(
                "INSERT INTO gen_metadata (idx, data, size) VALUES (?1, ?2, ?3)",
                rusqlite::params![idx, blob, 1],
            )
            .unwrap();
        }
    }

    // --- Aug 2026 layout: no absolute timestamp in the blob, wall-clock
    // times live in the steps table and are joined via request id ---

    fn new_usage_msg(input: u64, output: u64, cache: u64, request_id: &str) -> Vec<u8> {
        let mut m = vec![];
        varint_field(&mut m, 2, input);
        varint_field(&mut m, 3, output);
        if cache > 0 {
            varint_field(&mut m, 5, cache);
        }
        bytes_field(&mut m, 11, request_id.as_bytes());
        m
    }

    fn new_gen_blob(counter: u64, input: u64, output: u64, cache: u64, request_id: &str, model: &str) -> Vec<u8> {
        let mut t10 = vec![];
        varint_field(&mut t10, 1, counter);
        varint_field(&mut t10, 4, 256_000);
        let mut timing = vec![];
        varint_field(&mut timing, 2, u64::MAX); // sentinel; must not parse as a ts
        bytes_field(&mut timing, 10, &t10);
        let mut payload = vec![];
        bytes_field(&mut payload, 4, &new_usage_msg(input, output, cache, request_id));
        bytes_field(&mut payload, 9, &timing);
        bytes_field(&mut payload, 19, model.as_bytes());
        let mut blob = vec![];
        bytes_field(&mut blob, 1, &payload);
        blob
    }

    /// steps metadata: start/end Timestamps plus the LLM call info whose
    /// request id the gen row joins on. `end_sec = 0` omits a real end time;
    /// an empty `request_id` omits the call info entirely.
    fn step_meta(start_sec: u64, end_sec: u64, request_id: &str) -> Vec<u8> {
        let mut m = vec![];
        bytes_field(&mut m, 1, &ts_msg(start_sec, 250_000_000));
        if end_sec > 0 {
            bytes_field(&mut m, 32, &ts_msg(end_sec, 250_000_000));
        }
        if !request_id.is_empty() {
            let mut call = vec![];
            bytes_field(&mut call, 11, request_id.as_bytes());
            bytes_field(&mut m, 9, &call);
        }
        m
    }

    fn seed_new(home: &Path) {
        let dir = home.join(".gemini/antigravity/conversations");
        std::fs::create_dir_all(&dir).unwrap();
        let conn = rusqlite::Connection::open(dir.join("11111111-2222-3333-4444-555555555555.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE gen_metadata (idx integer, data blob, size integer NOT NULL DEFAULT 0, PRIMARY KEY (idx));
             CREATE TABLE steps (idx integer, metadata blob, PRIMARY KEY (idx));
             CREATE TABLE trajectory_metadata_blob (id text DEFAULT 'main', data blob, PRIMARY KEY (id));",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO trajectory_metadata_blob (id, data) VALUES ('main', ?1)",
            rusqlite::params![traj_blob("/Users/x/dev/newproj")],
        )
        .unwrap();
        for (idx, blob) in [
            (0, new_gen_blob(26_343, 900, 120, 0, "req_a", "claude-opus-4-6-thinking")),
            (1, new_gen_blob(35_922, 500, 60, 7000, "req_b", "gemini-3.7-flash")),
            (2, new_gen_blob(39_011, 300, 40, 0, "req_c", "gemini-3.7-flash")), // no matching step
        ] {
            conn.execute(
                "INSERT INTO gen_metadata (idx, data, size) VALUES (?1, ?2, ?3)",
                rusqlite::params![idx, blob, 1],
            )
            .unwrap();
        }
        for (idx, meta) in [
            (0, step_meta(1_755_000_000, 0, "")), // non-LLM step: no call info
            (2, step_meta(1_755_000_000, 1_755_000_004, "req_a")),
            (3, step_meta(1_755_000_010, 0, "req_b")), // end timestamp missing
        ] {
            conn.execute("INSERT INTO steps (idx, metadata) VALUES (?1, ?2)", rusqlite::params![idx, meta])
                .unwrap();
        }
    }

    #[test]
    fn parses_gen_metadata_tokens() {
        let home = test_home("antigravity");
        let store = test_store("antigravity");
        seed(&home);
        assert_eq!(collect(&store, &home).unwrap(), 2);
        let (model, input, output, reasoning, cache, provider, project, ts): (
            String, i64, i64, Option<i64>, i64, Option<String>, Option<String>, i64,
        ) = store
            .conn()
            .query_row(
                "SELECT model, input_tokens, output_tokens, reasoning_tokens, cache_read_tokens,
                        provider, project, ts FROM usage_event WHERE source_event_id LIKE '%:0'",
                [],
                |r| {
                    Ok((
                        r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(model, "gemini-3.7-flash");
        assert_eq!(input, 1000);
        assert_eq!(output, 200); // 300 total - 100 thinking
        assert_eq!(reasoning, Some(100));
        assert_eq!(cache, 5000);
        assert_eq!(provider.as_deref(), Some("google"));
        assert_eq!(project.as_deref(), Some("/Users/x/dev/proj"));
        assert_eq!(ts, 1_700_000_000_250);
    }

    #[test]
    fn is_idempotent_and_incremental() {
        let home = test_home("antigravity_idem");
        let store = test_store("antigravity_idem");
        seed(&home);
        assert_eq!(collect(&store, &home).unwrap(), 2);
        assert_eq!(collect(&store, &home).unwrap(), 0);
        // append a new gen row: only it is picked up
        let db = home
            .join(".gemini/antigravity/conversations/11111111-2222-3333-4444-555555555555.db");
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute(
            "INSERT INTO gen_metadata (idx, data, size) VALUES (3, ?1, 1)",
            rusqlite::params![gen_blob(1_700_000_300, 500, 50, 0, 0, "gemini-3.7-flash")],
        )
        .unwrap();
        drop(conn);
        assert_eq!(collect(&store, &home).unwrap(), 1);
        assert_eq!(store.conn().query_row("SELECT COUNT(*) FROM usage_event", [], |r| r.get::<_, i64>(0)).unwrap(), 3);
    }

    #[test]
    fn new_format_rows_join_to_steps_for_ts() {
        let home = test_home("antigravity_new");
        let store = test_store("antigravity_new");
        seed_new(&home);
        assert_eq!(collect(&store, &home).unwrap(), 3);

        let (ts, dur, model, out, cache): (i64, Option<i64>, String, i64, i64) = store
            .conn()
            .query_row(
                "SELECT ts, duration_ms, model, output_tokens, cache_read_tokens
                 FROM usage_event WHERE source_event_id LIKE '%:0'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap();
        assert_eq!(ts, 1_755_000_000_250); // step start, not the blob's counters
        assert_eq!(dur, Some(4_000)); // step end - start
        assert_eq!(model, "claude-opus-4-6-thinking");
        assert_eq!(out, 120); // no thinking field: output_total stays whole
        assert_eq!(cache, 0);

        // step without an end timestamp: ts resolves, duration stays empty
        let (ts, dur, cache): (i64, Option<i64>, i64) = store
            .conn()
            .query_row(
                "SELECT ts, duration_ms, cache_read_tokens FROM usage_event WHERE source_event_id LIKE '%:1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(ts, 1_755_000_010_250);
        assert_eq!(dur, None);
        assert_eq!(cache, 7000);

        // request id with no matching step falls back to the file's mtime
        let ts: i64 = store
            .conn()
            .query_row(
                "SELECT ts FROM usage_event WHERE source_event_id LIKE '%:2'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        assert!(ts > now_ms - 60_000 && ts <= now_ms + 1_000, "expected mtime, got {ts}");

        assert_eq!(collect(&store, &home).unwrap(), 0);
    }

    #[test]
    fn stale_watermarks_reset_once_after_format_bump() {
        let home = test_home("antigravity_upgrade");
        let store = test_store("antigravity_upgrade");
        seed_new(&home);
        // the pre-fix parser read these rows, inserted nothing, and still
        // advanced the watermark past them
        store.set_watermark("antigravity:11111111-2222-3333-4444-555555555555", 3);
        assert_eq!(collect(&store, &home).unwrap(), 3);
        assert_eq!(store.get_watermark("antigravity_format"), 2);

        // once the marker is current a watermark past the data is respected
        // again (normal incremental behavior)
        store.set_watermark("antigravity:11111111-2222-3333-4444-555555555555", 4);
        assert_eq!(collect(&store, &home).unwrap(), 0);
    }

    #[test]
    fn skips_truncated_and_unknown_blobs() {
        // wire reader must tolerate junk instead of panicking
        assert!(parse_gen(&[]).is_none());
        assert!(parse_gen(&[0xff]).is_none());
        assert!(parse_gen(&[0x0a, 0x05, b'a', b'b', b'c']).is_none()); // field1 too short
        let mut with_ts = vec![];
        let mut timing = vec![];
        bytes_field(&mut timing, 4, &ts_msg(42, 0));
        bytes_field(&mut with_ts, 9, &timing);
        let mut blob = vec![];
        bytes_field(&mut blob, 1, &with_ts);
        let g = parse_gen(&blob).unwrap();
        assert_eq!(g.ts, 42_000);
        assert_eq!(g.input + g.output_total + g.cache_read, 0);
    }

    #[test]
    fn timestamp_saturates_on_absurd_values() {
        // u64::MAX serializes as a 10-byte varint; must not panic, loop, or wrap
        let mut m = vec![];
        varint_field(&mut m, 1, u64::MAX);
        assert_eq!(timestamp_ms(&m), i64::MAX);
    }

    /// Opt-in smoke test against the developer's real Antigravity data:
    ///   ANTIGRAVITY_REAL_HOME=$HOME cargo test antigravity -- --ignored
    #[test]
    #[ignore]
    fn real_data_smoke() {
        let Ok(home) = std::env::var("ANTIGRAVITY_REAL_HOME") else { return };
        let store = test_store("antigravity_real");
        let n = collect(&store, Path::new(&home)).unwrap();
        let count: i64 = store
            .conn()
            .query_row("SELECT COUNT(*) FROM usage_event WHERE source='antigravity'", [], |r| r.get(0))
            .unwrap();
        assert!(n > 0, "expected events from real data, got {n}");
        assert_eq!(n as i64, count, "first run should insert everything once");
        assert_eq!(collect(&store, Path::new(&home)).unwrap(), 0, "second run must be a no-op");
        let (sum_in, sum_out): (i64, i64) = store
            .conn()
            .query_row(
                "SELECT SUM(input_tokens), SUM(output_tokens) FROM usage_event WHERE source='antigravity'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        println!("real data: {count} events, in={sum_in}, out={sum_out}");
    }
}
