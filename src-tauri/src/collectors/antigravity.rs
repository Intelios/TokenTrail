use crate::models::{Source, UsageEvent};
use crate::store::{open_readonly, Store};
use std::path::Path;

/// Google Antigravity stores one SQLite DB per conversation under
/// ~/.gemini/antigravity/conversations/<uuid>.db. LLM calls live in the
/// `gen_metadata` table as protobuf blobs (the Windsurf-lineage
/// ChatModelMetadata message). The wire layout was mapped against local data:
///   field 1 (message) ->
///     4:  ModelUsageStats  { 2: input, 3: output_total, 5: cache_read,
///                            9: thinking, 10: response_output }
///     9:  timing          { 4: start Timestamp{1: s, 2: ns}, 8: elapsed Duration }
///     19: model id string (e.g. "gemini-3.7-flash")
/// `output_total` is thinking + response_output; per repo convention reasoning
/// is kept out of output_tokens. Rows are append-only with a contiguous `idx`,
/// so a per-file watermark on idx keeps re-ingestion cheap and idempotent.
/// Sub-trajectories (agent tasks) currently surface as their own conversation
/// DBs; there is no in-file subagent marker to read.
pub fn collect(store: &Store, home: &Path) -> Result<usize, String> {
    let root = home.join(".gemini/antigravity/conversations");
    if !root.exists() {
        return Ok(0);
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
            let thinking = g.thinking.min(g.output_total);
            events.push(UsageEvent {
                source: Source::Antigravity,
                source_event_id: format!("{uuid}:{idx}"),
                ts: g.ts,
                session_id: Some(uuid.to_string()),
                project: project.clone(),
                provider: provider_for(g.model.as_deref()),
                model: g.model,
                input_tokens: g.input,
                output_tokens: g.output_total - thinking,
                reasoning_tokens: (thinking > 0).then_some(thinking),
                cache_read_tokens: g.cache_read,
                cache_write_tokens: 0,
                duration_ms: g.duration_ms,
                ttft_ms: None,
                is_subagent: false,
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

fn parse_gen(blob: &[u8]) -> Option<GenCall> {
    let mut g = GenCall {
        ts: 0,
        input: 0,
        output_total: 0,
        thinking: 0,
        cache_read: 0,
        duration_ms: None,
        model: None,
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
    (g.ts > 0).then_some(g)
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
