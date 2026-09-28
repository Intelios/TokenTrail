//! Purpose-built, usage-only WackCode ledger. Never open conversation files.
use super::sorted_glob;
use crate::{
    models::{Source, UsageEvent},
    store::Store,
};
use serde::Deserialize;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

pub fn root(home: &Path) -> PathBuf {
    home.join("Library/Application Support/com.wackcode.desktop/usage/v1")
}
const MAX_BATCH: u64 = 4 * 1024 * 1024;
#[derive(Deserialize)]
struct Tokens {
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
}
#[derive(Deserialize)]
struct Record {
    v: u32,
    id: String,
    ts: i64,
    duration_ms: i64,
    session_id: String,
    project: Option<String>,
    workspace: Option<String>,
    provider: String,
    model: String,
    purpose: String,
    subagent_id: Option<String>,
    outcome: String,
    tokens: Option<Tokens>,
}
fn parse(line: &str) -> Result<Option<UsageEvent>, String> {
    let r: Record = serde_json::from_str(line).map_err(|_| "Malformed usage record.")?;
    if r.v != 1 {
        return Err(format!("Unsupported WackCode usage version {}.", r.v));
    }
    if r.id.is_empty()
        || r.session_id.is_empty()
        || r.model.is_empty()
        || r.provider.is_empty()
        || r.ts <= 0
        || r.duration_ms < 0
        || ![
            "chat",
            "subagent",
            "title",
            "compaction",
            "branch_summary",
            "goal_verification",
            "commit_message",
        ]
        .contains(&r.purpose.as_str())
        || !["completed", "failed", "cancelled"].contains(&r.outcome.as_str())
    {
        return Err("Invalid usage metadata.".into());
    }
    let Some(t) = r.tokens else { return Ok(None) };
    if [t.input, t.output, t.cache_read, t.cache_write]
        .iter()
        .any(|n| *n < 0 || *n > 9_007_199_254_740_991)
    {
        return Err("Invalid usage token counts.".into());
    }
    Ok(Some(UsageEvent {
        source: Source::WackCode,
        source_event_id: r.id,
        ts: r.ts,
        session_id: Some(r.session_id),
        project: r.project,
        provider: Some(r.provider),
        model: Some(r.model),
        input_tokens: t.input,
        output_tokens: t.output,
        reasoning_tokens: None,
        cache_read_tokens: t.cache_read,
        cache_write_tokens: t.cache_write,
        duration_ms: Some(r.duration_ms),
        ttft_ms: None,
        is_subagent: r.subagent_id.is_some(),
        estimated: false,
        purpose: Some(r.purpose),
        outcome: Some(r.outcome),
        workspace: r.workspace,
        subagent_id: r.subagent_id,
    }))
}

pub fn collect(store: &Store, home: &Path) -> Result<usize, String> {
    let mut processed = 0;
    let mut errors = Vec::new();
    for path in sorted_glob(&format!("{}/*/*.jsonl", root(home).display()))? {
        let result = collect_file(store, &path);
        match result {
            Ok(n) => processed += n,
            Err(e) => errors.push(e),
        }
    }
    if errors.is_empty() {
        Ok(processed)
    } else {
        Err(format!(
            "WackCode: {} segment(s) need attention. {}",
            errors.len(),
            errors[0]
        ))
    }
}
fn collect_file(store: &Store, path: &Path) -> Result<usize, String> {
    let mut file = File::open(path).map_err(|_| "Could not read usage segment.")?;
    let meta = file
        .metadata()
        .map_err(|_| "Could not inspect usage segment.")?;
    // File identity detects replacement even when the new file is longer than our cursor.
    #[cfg(unix)]
    let identity = format!("{}:{}", meta.dev(), meta.ino());
    #[cfg(not(unix))]
    let identity = format!("{:?}", meta.created().ok());
    let key = format!("{}:{identity}", path.display());
    let mut offset = store.get_offset("wackcode", &key).unwrap_or(0);
    if meta.len() < offset {
        offset = 0;
    }
    // Verify the bytes immediately before the cursor as well, for in-place rewrites.
    if offset > 0 {
        let hash = boundary_hash(&mut file, offset)?;
        if store.get_watermark(&format!("wackcode:boundary:{key}")) != hash {
            offset = 0;
        }
    }
    file.seek(SeekFrom::Start(offset))
        .map_err(|_| "Could not seek usage segment.")?;
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_BATCH.min(meta.len().saturating_sub(offset)))
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read usage segment.")?;
    let Some(end) = bytes.iter().rposition(|b| *b == b'\n') else {
        if bytes.len() as u64 == MAX_BATCH {
            return Err("Usage record exceeds the read limit.".into());
        }
        return Ok(0);
    };
    let mut events = Vec::new();
    let mut error = None;
    let mut consumed = 0;
    for line in bytes[..=end].split_inclusive(|b| *b == b'\n') {
        let parsed = std::str::from_utf8(line)
            .map_err(|_| "Invalid UTF-8 usage record.".to_string())
            .and_then(parse);
        match parsed {
            Ok(Some(event)) => events.push(event),
            Ok(None) => {}
            Err(e) => {
                error = Some(e);
                break;
            }
        }
        consumed += line.len() as u64;
    }
    let n = store
        .insert_events(&events)
        .map_err(|_| "Could not import WackCode usage.")?;
    let next = offset + consumed;
    // A failed cursor update causes a safe replay; event IDs make ingestion idempotent.
    store.set_watermark(
        &format!("wackcode:boundary:{key}"),
        boundary_hash(&mut file, next)?,
    );
    store.set_offset("wackcode", &key, next);
    if let Some(e) = error {
        Err(e)
    } else {
        Ok(n)
    }
}
fn boundary_hash(file: &mut File, offset: u64) -> Result<i64, String> {
    let start = offset.saturating_sub(256);
    file.seek(SeekFrom::Start(start))
        .map_err(|_| "Could not verify usage cursor.")?;
    let mut bytes = vec![0; (offset - start) as usize];
    file.read_exact(&mut bytes)
        .map_err(|_| "Could not verify usage cursor.")?;
    Ok(bytes.into_iter().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    }) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collectors::{test_home, test_store};
    use std::{fs, io::Write};
    const FIXTURE: &str = include_str!("../../fixtures/wackcode_usage_v1.jsonl");
    fn setup(tag: &str) -> (PathBuf, Store, PathBuf) {
        let home = test_home(tag);
        let store = test_store(tag);
        let dir = root(&home).join("2026-09");
        fs::create_dir_all(&dir).unwrap();
        (home, store, dir.join("a.jsonl"))
    }
    fn count(store: &Store) -> i64 {
        store
            .read_conn()
            .query_row("SELECT COUNT(*) FROM usage_event", [], |r| r.get(0))
            .unwrap()
    }
    #[test]
    fn producer_fixture_imports_once_and_breakdown_reconciles() {
        let (home, store, file) = setup("wackcode-fixture");
        fs::write(&file, FIXTURE).unwrap();
        assert_eq!(collect(&store, &home).unwrap(), 7);
        assert_eq!(collect(&store, &home).unwrap(), 0);
        assert_eq!(count(&store), 7);
        let rows = crate::aggregate::wackcode_usage(&store, 3650).unwrap();
        assert_eq!(rows.len(), 7);
        assert_eq!(rows.iter().map(|r| r.tokens).sum::<i64>(), 140);
        let children: i64 = store
            .read_conn()
            .query_row(
                "SELECT COUNT(*) FROM usage_event WHERE is_subagent=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(children, 2);
        store
            .read_conn()
            .execute("INSERT INTO hidden_model VALUES ('claude-sonnet-4-5')", [])
            .unwrap();
        assert!(crate::aggregate::wackcode_usage(&store, 3650).unwrap().is_empty());
    }
    #[test]
    fn waits_for_tail_and_handles_replacement_truncation_and_many_writers() {
        let (home, store, file) = setup("wackcode-tail");
        let first = FIXTURE.lines().next().unwrap();
        fs::write(&file, &first[..first.len() / 2]).unwrap();
        assert_eq!(collect(&store, &home).unwrap(), 0);
        let mut f = fs::OpenOptions::new().append(true).open(&file).unwrap();
        writeln!(f, "{}", &first[first.len() / 2..]).unwrap();
        assert_eq!(collect(&store, &home).unwrap(), 1);
        fs::write(&file, FIXTURE).unwrap();
        collect(&store, &home).unwrap();
        assert_eq!(count(&store), 7);
        fs::remove_file(&file).unwrap();
        fs::write(&file, FIXTURE).unwrap();
        fs::write(file.with_file_name("b.jsonl"), FIXTURE).unwrap();
        collect(&store, &home).unwrap();
        assert_eq!(count(&store), 7);
        fs::write(&file, format!("{first}\n")).unwrap();
        collect(&store, &home).unwrap();
        assert_eq!(count(&store), 7);
    }
    #[test]
    fn validates_versions_and_counts_and_excludes_unknown_usage() {
        let mut value: serde_json::Value =
            serde_json::from_str(FIXTURE.lines().next().unwrap()).unwrap();
        value["tokens"] = serde_json::Value::Null;
        assert!(parse(&value.to_string()).unwrap().is_none());
        value["v"] = 2.into();
        assert!(parse(&value.to_string()).unwrap_err().contains("version"));
        value["v"] = 1.into();
        value["tokens"] = serde_json::json!({"input":-1,"output":0,"cache_read":0,"cache_write":0});
        assert!(parse(&value.to_string()).is_err());
        let (home, store, file) = setup("wackcode-invalid");
        fs::write(&file, "{broken}\n").unwrap();
        fs::write(file.with_file_name("b.jsonl"), FIXTURE).unwrap();
        assert!(collect(&store, &home).is_err());
        assert_eq!(count(&store), 7);
    }
    #[test]
    fn migration_is_idempotent_and_preserves_existing_rows() {
        let home = test_home("wackcode-migration");
        let path = home.join("old.db");
        let store = Store::open(&path).unwrap();
        store
            .insert_events(&[parse(FIXTURE.lines().next().unwrap()).unwrap().unwrap()])
            .unwrap();
        for col in ["purpose", "outcome", "workspace", "subagent_id"] {
            store
                .read_conn()
                .execute(&format!("ALTER TABLE usage_event DROP COLUMN {col}"), [])
                .unwrap();
        }
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(count(&store), 1);
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(count(&store), 1);
        let purpose: Option<String> = store
            .read_conn()
            .query_row("SELECT purpose FROM usage_event", [], |r| r.get(0))
            .unwrap();
        assert_eq!(purpose, None);
    }
}
