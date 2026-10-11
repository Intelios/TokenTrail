use crate::collectors::{clean_model, parse_ts_ms, sorted_glob};
use crate::models::{Source, UsageEvent};
use crate::store::{open_readonly, Store, WackContext};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
};

/// WackChatter — a roleplay chat frontend for OpenAI-compatible providers.
///
/// Two readers, because the app records usage in two places for two reasons:
///
///   * `~/.wackchatter/usage/v2/YYYY-MM/*.jsonl` — one retained line per generation,
///     recording on by default (explicit off is respected). Legacy logs remain readable. The only source for the generations that never become a stored
///     message: Arena rounds, rolling summaries, memory extraction, persona derivation.
///   * `<library>/chats.db` — the transcripts themselves, which carry per-swipe token
///     counts going back to before any of this existed. The only source for history.
///
/// They overlap for ordinary replies, and deliberately so: the log covers what the
/// database cannot and the database covers when the log was not running. Both derive the
/// same `source_event_id` from the generation id WackChatter mints per request, so the
/// store's upsert collapses the overlap instead of double-counting it.
///
/// `~/.wackchatter/library.json` is how the database half is found at all — the library
/// moves, and the pointer is written outside it for exactly this reason.
const OVERLAP_MS: i64 = 5 * 60 * 1000;

pub fn collect(store: &Store, home: &Path) -> Result<usize, String> {
    let dir = home.join(".wackchatter");
    if !dir.exists() {
        return Ok(0);
    }
    // The database half never fails the sync: a library that has moved, been deleted, or
    // is mid-write is a normal state, and the log is still worth reading.
    let log = collect_log(store, &dir);
    let db = collect_db(store, &dir).unwrap_or(0);
    log.map(|n| n + db)
}

// ---------------------------------------------------------------------------
// The live log
// ---------------------------------------------------------------------------

/// Retained v2 segments plus both surviving legacy files; one cursor per file identity.
const MAX_BATCH: u64 = 4 * 1024 * 1024;
fn collect_log(store: &Store, dir: &Path) -> Result<usize, String> {
    let mut files = sorted_glob(&format!("{}/usage/v2/*/*.jsonl", dir.display()))?;
    for name in ["usage.jsonl.1", "usage.jsonl"] {
        let p = dir.join(name);
        if p.exists() {
            files.push(p);
        }
    }
    let mut total = 0;
    let mut errors = Vec::new();
    for path in files {
        match collect_file(store, &path) {
            Ok(n) => total += n,
            Err(e) => errors.push(e),
        }
    }
    if errors.is_empty() {
        Ok(total)
    } else {
        Err(errors.join("; "))
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
    let mut offset = store.get_offset("wackchatter:ledger-v2", &key).unwrap_or(0);
    if meta.len() < offset {
        offset = 0;
    }
    // Verify the bytes immediately before the cursor as well, for in-place rewrites.
    if offset > 0 {
        let hash = boundary_hash(&mut file, offset)?;
        if store.get_watermark(&format!("wackchatter:boundary:{key}")) != hash {
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
    let mut contexts = Vec::new();
    let mut error = None;
    let mut consumed = 0;
    for line in bytes[..=end].split_inclusive(|b| *b == b'\n') {
        let parsed = std::str::from_utf8(line)
            .map_err(|_| "Invalid UTF-8 usage record.".to_string())
            .and_then(|line| {
                let value: serde_json::Value = serde_json::from_str(line)
                    .map_err(|_| "Malformed WackChatter record".to_string())?;
                if !matches!(value["v"].as_i64(), Some(1 | 2)) {
                    return Err("Unsupported WackChatter usage version".to_string());
                }
                if let Some(event) = parse_log_line(line) {
                    contexts.push(WackContext {
                        rank: 2,
                        library_id: value["library_id"].as_str().map(String::from),
                        character_id: value["character"].as_str().map(String::from),
                        legacy_id: None,
                    });
                    Ok(Some(event))
                } else {
                    Ok(None)
                }
            });
        match parsed {
            Ok(Some(event)) => events.push(event),
            Ok(None) => {}
            Err(e) => {
                error.get_or_insert(e);
            }
        }
        consumed += line.len() as u64;
    }
    let n = store
        .insert_wackchatter(&events, &contexts)
        .map_err(|_| "Could not import WackChatter usage.")?;
    let next = offset + consumed;
    // A failed cursor update causes a safe replay; event IDs make ingestion idempotent.
    store.set_watermark(
        &format!("wackchatter:boundary:{key}"),
        boundary_hash(&mut file, next)?,
    );
    store.set_offset("wackchatter:ledger-v2", &key, next);
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

fn parse_log_line(line: &str) -> Option<UsageEvent> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    // Version gate rather than best-effort parsing: a future writer that changes what a
    // field means must not be read as if it still meant the old thing.
    if !matches!(v.get("v").and_then(|x| x.as_i64()), Some(1 | 2)) {
        return None;
    }
    let id = v
        .get("id")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())?;
    let ts = v.get("ts").and_then(|x| x.as_str()).and_then(parse_ts_ms)?;
    let num = |k: &str| {
        v.get(k)
            .and_then(|x| x.as_i64())
            .unwrap_or(0)
            .clamp(0, 9_007_199_254_740_991)
    };
    let (input, output) = (num("input_tokens"), num("output_tokens"));
    let (cr, cw) = (num("cache_read_tokens"), num("cache_write_tokens"));
    if input + output + cr + cw == 0 {
        return None; // a request that failed before the provider generated anything
    }
    let feature = v.get("feature").and_then(|x| x.as_str()).unwrap_or("chat");
    let str_of = |k: &str| v.get(k).and_then(|x| x.as_str()).map(String::from);

    Some(UsageEvent {
        source: Source::WackChatter,
        source_event_id: id.to_string(),
        ts,
        session_id: str_of("session_id"),
        project: project_for(v.get("character").and_then(|x| x.as_str())),
        provider: str_of("provider"),
        provider_name: str_of("provider_name"),
        model: v.get("model").and_then(|x| x.as_str()).map(clean_model),
        input_tokens: input,
        output_tokens: output,
        // Reported for visibility only. It is a subset of output_tokens, which is what
        // the store's token total counts — see the TOKENS constant in aggregate.rs.
        reasoning_tokens: Some(num("reasoning_tokens")).filter(|n| *n > 0),
        cache_read_tokens: cr,
        cache_write_tokens: cw,
        duration_ms: Some(num("duration_ms")).filter(|n| *n > 0),
        ttft_ms: v.get("ttft_ms").and_then(|x| x.as_i64()).filter(|n| *n > 0),
        is_subagent: is_background(feature),
        estimated: v.get("estimated").and_then(|x| x.as_bool()).unwrap_or(true),
        purpose: Some(feature.to_string()),
        outcome: str_of("outcome").or_else(|| {
            v["aborted"]
                .as_bool()
                .filter(|b| *b)
                .map(|_| "cancelled".into())
        }),
        workspace: None,
        subagent_id: None,
        mode: None,
    })
}

/// Work the app does for itself rather than a reply the user asked for.
///
/// The same idea as a harness sidechain: in service of the conversation, not part of it.
/// Co-Creator and Arena are excluded — a person is driving both of those.
fn is_background(feature: &str) -> bool {
    matches!(
        feature,
        "summary" | "memory" | "persona" | "groupDirector" | "groupProfiles"
    )
}

/// Roleplay has characters where coding has repositories, and the Projects page is the
/// place that answers "where did it go". Prefixed so a character can never be mistaken
/// for a checkout, and so everything from this source groups together when sorted.
fn project_for(character: Option<&str>) -> Option<String> {
    Some(match character.map(str::trim).filter(|c| !c.is_empty()) {
        Some(c) => format!("WackChatter · {}", c.trim_end_matches(".png")),
        None => "WackChatter".to_string(),
    })
}

// ---------------------------------------------------------------------------
// The transcripts
// ---------------------------------------------------------------------------

/// Which library WackChatter published on its last boot.
///
/// Only ever the *current* one, because that is all the pointer can say — it is rewritten
/// on every launch and a machine can hold several libraries. Reading only this one would
/// mean the rest stop being kept current the moment someone launches the other install.
pub(crate) fn pointed_library(dir: &Path) -> Option<PathBuf> {
    let raw = std::fs::read_to_string(dir.join("library.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let data_dir = PathBuf::from(v.get("dataDir").and_then(|x| x.as_str())?);
    data_dir.join("chats.db").exists().then_some(data_dir)
}

/// A library's identity, for keying its read cursor.
///
/// One cursor per library, not one per source. There can be more than one library on a
/// machine — a second checkout, a copy on an external drive — and the pointer names
/// whichever booted last. Sharing a cursor between them means the newer library's
/// timestamp hides the older one's history completely, and it never comes back, because
/// nothing about the skipped library will ever be modified later than a cursor that has
/// already passed it.
///
/// `.wackchatter` marks a folder as a library and records the moment it became one —
/// the only thing about it that survives the folder being moved or renamed. The path is
/// the fallback: never wrong, just re-reads everything after a move.
pub(crate) fn library_key(data_dir: &Path) -> String {
    std::fs::read_to_string(data_dir.join(".wackchatter"))
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|v| v.get("created").and_then(|c| c.as_str()).map(String::from))
        .unwrap_or_else(|| data_dir.display().to_string())
}

/// One SQL shape for both transcript tables.
///
/// `json_each` over `swipe_info` yields one row per swipe, which is one row per
/// generation — a re-roll is a separate request and a separate charge. User turns and
/// position 0 are excluded for the reason WackChatter's own stats exclude them: a
/// greeting carries no model and a card's alternate greetings arrive as swipes on it.
fn swipe_query(table: &str, parent: &str, owner: &str, extra_where: &str) -> String {
    format!(
        "SELECT p.id, m.id, s.key,
                json_extract(s.value, '$.extra.generation_id'),
                json_extract(s.value, '$.extra.model'),
                json_extract(s.value, '$.extra.api'),
                json_extract(s.value, '$.extra.token_count'),
                json_extract(s.value, '$.extra.prompt_tokens'),
                json_extract(s.value, '$.extra.usage_reported'),
                json_extract(s.value, '$.gen_started'),
                json_extract(s.value, '$.gen_finished'),
                json_extract(s.value, '$.send_date'),
                {owner},
                p.modified
         FROM {table} m, json_each(m.swipe_info) s, {parent} p
         WHERE p.id = m.{parent_key} AND m.is_user = 0 AND m.position > 0
           AND p.modified >= ?1 {extra_where}",
        parent_key = if parent == "chats" {
            "chat_id"
        } else {
            "session_id"
        },
    )
}

/// One swipe as SQLite hands it back. Every JSON extraction is nullable: these rows were
/// written by several versions of the app, and the oldest of them predate most of it.
type SwipeRow = (
    String,         // parent id (chat / session)
    String,         // message id
    i64,            // swipe index
    Option<String>, // generation_id
    Option<String>, // model
    Option<String>, // api (provider)
    Option<i64>,    // token_count  (completion side)
    Option<i64>,    // prompt_tokens
    Option<bool>,   // usage_reported
    Option<String>, // gen_started
    Option<String>, // gen_finished
    Option<String>, // send_date
    Option<String>, // character (chats only)
    i64,            // parent `modified` — the cursor column, not an event field
);

/// Read every library this machine has ever pointed at, not only the current one.
///
/// The pointer names whichever install booted last, so a second checkout — or a copy on
/// a drive — would otherwise go stale the moment the other one is launched. Registering
/// each library as it appears and reading all of them makes "WackChatter" one source
/// regardless of how many libraries are behind it. Each keeps its own cursor, so this
/// costs one cheap no-op query per idle library per sync.
fn collect_db(store: &Store, dir: &Path) -> Option<usize> {
    const REGISTRY: &str = "wackchatter:library";
    if let Some(current) = pointed_library(dir) {
        store.remember_path(REGISTRY, &current.display().to_string());
    }
    let mut processed = 0usize;
    for path in store.known_paths(REGISTRY) {
        processed += collect_library(store, Path::new(&path)).unwrap_or(0);
    }
    Some(processed)
}

/// One library. `None` when it cannot be opened — a folder that has been deleted, or a
/// drive that is not plugged in right now. Neither is an error worth failing a sync over.
fn collect_library(store: &Store, data_dir: &Path) -> Option<usize> {
    let db = data_dir.join("chats.db");
    if !db.exists() {
        return None;
    }
    let conn = open_readonly(&db).ok()?;
    let library = library_key(data_dir);
    store
        .remember_wackchatter_library(&data_dir.to_string_lossy(), &library)
        .ok()?;

    let mut processed = 0usize;
    // Two cursors per library, because the two tables advance independently: a week of
    // Co-Creator work must not be skipped because a chat was touched more recently.
    for (table, parent, owner, prefix, scope) in [
        ("messages", "chats", "p.character_id", "wcd", "chats"),
        (
            "cocreator_messages",
            "cocreator_sessions",
            "NULL",
            "wcc",
            "cocreator",
        ),
    ] {
        let key = &format!("wackchatter:{scope}:{library}");
        // Overlapping the watermark covers the window between a swipe being written and
        // its chat's `modified` settling, the same guard opencode.rs uses.
        let upgraded = store.get_watermark(&format!("{key}:context-v2")) > 0;
        let watermark = if upgraded {
            store.get_watermark(key).saturating_sub(OVERLAP_MS)
        } else {
            0
        };
        // Co-Creator rows carry no is_system column — the table has two speakers and no
        // hiding — so that filter belongs only to the chat transcripts.
        let extra = if table == "messages" {
            "AND m.is_system = 0"
        } else {
            ""
        };

        let has_character = conn
            .prepare("SELECT name FROM pragma_table_info('messages') WHERE name='character_id'")
            .and_then(|mut s| s.exists([]))
            .unwrap_or(false);
        let owner = if table == "messages" && has_character {
            "COALESCE(m.character_id,p.character_id)"
        } else {
            owner
        };
        let Ok(mut stmt) = conn.prepare(&swipe_query(table, parent, owner, extra)) else {
            continue; // an older library without this table
        };
        let Ok(rows) = stmt.query_map([watermark], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
                r.get(8)?,
                r.get(9)?,
                r.get(10)?,
                r.get(11)?,
                r.get::<_, Option<String>>(12)?,
                r.get(13)?,
            ))
        }) else {
            continue;
        };

        let mut events = Vec::new();
        let mut contexts = Vec::new();
        // The cursor has to be over the column the query filters on. Advancing it by the
        // event's own timestamp instead compares two different clocks — `gen_finished` is
        // when the provider stopped writing, `modified` is when the chat was next saved —
        // and a library whose two clocks are far apart is then read wrongly forever.
        // Advanced for skipped rows too, or an unreadable swipe is rescanned every sync.
        let mut max_modified = 0i64;
        for row in rows.flatten() {
            max_modified = max_modified.max(row.13);
            let character = row.12.clone();
            let Some(mut ev) = from_swipe(row, prefix) else {
                continue;
            };
            let legacy = ev
                .source_event_id
                .starts_with(&format!("{prefix}:"))
                .then(|| ev.source_event_id.clone());
            if legacy.is_some() {
                ev.source_event_id = format!("{}:{}", library, ev.source_event_id);
            }
            ev.purpose = Some(
                if table == "messages" {
                    "chat"
                } else {
                    "cocreator"
                }
                .into(),
            );
            contexts.push(WackContext {
                rank: 1,
                library_id: Some(library.clone()),
                character_id: character,
                legacy_id: legacy,
            });
            events.push(ev);
        }
        processed += store.insert_wackchatter(&events, &contexts).ok()?;
        store.set_watermark(&format!("{key}:context-v2"), 1);
        if max_modified > 0 {
            store.set_watermark(key, max_modified);
        }
    }
    Some(processed)
}

fn from_swipe(row: SwipeRow, prefix: &str) -> Option<UsageEvent> {
    let (
        parent,
        message,
        swipe,
        gen_id,
        model,
        api,
        out,
        prompt,
        reported,
        started,
        finished,
        sent,
        character,
        _modified,
    ) = row;

    let output = out.unwrap_or(0);
    let input = prompt.unwrap_or(0);
    if output + input == 0 {
        return None; // a swipe from before anything was counted at all
    }

    // The generation id when there is one, so this row and its live-log twin are the same
    // event. Everything written before ids existed falls back to its position in the
    // transcript, which is stable and cannot collide with a uuid.
    let source_event_id = match gen_id.filter(|s| !s.is_empty()) {
        Some(id) => id,
        None => format!("{prefix}:{parent}:{message}:{swipe}"),
    };

    // When it finished, not when the turn began: that is the moment the tokens existed.
    let ts = finished
        .as_deref()
        .and_then(parse_ts_ms)
        .or_else(|| sent.as_deref().and_then(parse_ts_ms))?;

    let duration_ms = match (
        started.as_deref().and_then(parse_ts_ms),
        finished.as_deref().and_then(parse_ts_ms),
    ) {
        (Some(a), Some(b)) if b > a => Some(b - a),
        _ => None,
    };

    Some(UsageEvent {
        source: Source::WackChatter,
        source_event_id,
        ts,
        session_id: Some(parent),
        project: project_for(character.as_deref()),
        provider: api,
        provider_name: None,
        model: model.as_deref().map(clean_model),
        // Prompt tokens are only ever written when the provider reported them, so there
        // is no cached figure to subtract here — the client already did it.
        input_tokens: input,
        output_tokens: output,
        reasoning_tokens: None,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        duration_ms,
        ttft_ms: None,
        is_subagent: false,
        // The flag is only written from 2026-08 onwards. Its absence means the count came
        // from the app's own tokenizer, which is what every older row holds.
        estimated: !reported.unwrap_or(false),
        purpose: None,
        outcome: None,
        workspace: None,
        subagent_id: None,
        mode: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collectors::{fixture, test_home, test_store};

    const LINE: &str = r#"{"v":1,"id":"gen-1","ts":"2026-08-31T10:00:05.000Z","feature":"chat","session_id":"chat-a","character":"Ayla.png","provider":"openrouter","connection_id":"c1","model":"anthropic/claude-sonnet-5","input_tokens":8421,"output_tokens":612,"reasoning_tokens":200,"cache_read_tokens":4096,"cache_write_tokens":0,"duration_ms":9210,"ttft_ms":640,"estimated":false,"aborted":false}"#;

    fn seed_log(home: &Path, body: &str) {
        let dir = home.join(".wackchatter");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("usage.jsonl"), body).unwrap();
    }

    #[test]
    fn reads_a_log_line_and_does_not_re_read_it() {
        let home = test_home("wc-log");
        let store = test_store("wc-log");
        seed_log(&home, &format!("{LINE}\n"));

        assert_eq!(collect(&store, &home).unwrap(), 1);
        let (id, input, output, cr, reasoning, ttft, est): (String, i64, i64, i64, i64, i64, i64) =
            store
                .read_conn()
                .query_row(
                    "SELECT source_event_id, input_tokens, output_tokens, cache_read_tokens,
                        COALESCE(reasoning_tokens,0), COALESCE(ttft_ms,0), estimated
                 FROM usage_event",
                    [],
                    |r| {
                        Ok((
                            r.get(0)?,
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            r.get(4)?,
                            r.get(5)?,
                            r.get(6)?,
                        ))
                    },
                )
                .unwrap();
        assert_eq!(
            (id.as_str(), input, output, cr, reasoning, ttft, est),
            ("gen-1", 8421, 612, 4096, 200, 640, 0)
        );

        // The OpenRouter vendor prefix is stripped so pricing and families can match.
        let (model, project, session): (String, String, String) = store
            .read_conn()
            .query_row(
                "SELECT model, project, session_id FROM usage_event",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(model, "claude-sonnet-5");
        assert_eq!(project, "WackChatter · Ayla");
        assert_eq!(session, "chat-a");

        assert_eq!(collect(&store, &home).unwrap(), 0);
    }

    #[test]
    fn skips_other_versions_empty_generations_and_partial_lines() {
        let home = test_home("wc-skip");
        let store = test_store("wc-skip");
        let zero = LINE
            .replace("\"input_tokens\":8421", "\"input_tokens\":0")
            .replace("\"output_tokens\":612", "\"output_tokens\":0")
            .replace("\"cache_read_tokens\":4096", "\"cache_read_tokens\":0")
            .replace("\"gen-1\"", "\"gen-zero\"");
        let v2 = LINE
            .replace("\"v\":1", "\"v\":99")
            .replace("\"gen-1\"", "\"gen-v2\"");
        // The last line has no newline: a half-written tail must be retried, not parsed.
        let partial = LINE.replace("\"gen-1\"", "\"gen-partial\"");
        seed_log(&home, &format!("{LINE}\n{zero}\n{v2}\nnot json\n{partial}"));

        assert!(collect(&store, &home).unwrap_err().contains("Unsupported"));
        let ids: Vec<String> = store
            .read_conn()
            .prepare("SELECT source_event_id FROM usage_event")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(ids, vec!["gen-1"]);

        // Completing the partial line is what makes it readable.
        seed_log(
            &home,
            &format!("{LINE}\n{zero}\n{v2}\nnot json\n{partial}\n"),
        );
        assert_eq!(collect(&store, &home).unwrap(), 1);
    }

    #[test]
    fn background_features_are_flagged_and_estimates_default_to_true() {
        let summary = LINE
            .replace("\"feature\":\"chat\"", "\"feature\":\"summary\"")
            .replace(",\"estimated\":false", "");
        let ev = parse_log_line(&summary).unwrap();
        assert!(ev.is_subagent);
        // A record that does not say is a record that did not know.
        assert!(ev.estimated);

        let arena = LINE.replace("\"feature\":\"chat\"", "\"feature\":\"arena\"");
        assert!(!parse_log_line(&arena).unwrap().is_subagent);
    }

    #[test]
    fn a_character_less_generation_still_lands_under_the_source() {
        let line = LINE.replace(",\"character\":\"Ayla.png\"", "");
        assert_eq!(
            parse_log_line(&line).unwrap().project.as_deref(),
            Some("WackChatter")
        );
    }

    /// A minimal WackChatter library: the two transcript tables and a pointer to them.
    fn seed_library(home: &Path, swipes: &str) -> PathBuf {
        seed_named_library(home, "Library", "chat-a", swipes, 1756634405000)
    }

    fn seed_named_library(
        home: &Path,
        name: &str,
        chat: &str,
        swipes: &str,
        modified: i64,
    ) -> PathBuf {
        let lib = home.join(name);
        std::fs::create_dir_all(&lib).unwrap();
        // The marker is what tells two libraries apart; `created` is unique to each.
        std::fs::write(
            lib.join(".wackchatter"),
            format!("{{\"app\":\"wackchatter\",\"version\":1,\"created\":\"{name}\"}}"),
        )
        .unwrap();
        let db = lib.join("chats.db");
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE chats (id TEXT PRIMARY KEY, character_id TEXT NOT NULL, title TEXT,
                                 created INTEGER NOT NULL, modified INTEGER NOT NULL,
                                 revision INTEGER, metadata TEXT);
             CREATE TABLE messages (chat_id TEXT NOT NULL, id TEXT NOT NULL, position INTEGER NOT NULL,
                                    name TEXT, is_user INTEGER NOT NULL, is_system INTEGER NOT NULL,
                                    swipe_id INTEGER, swipes TEXT NOT NULL, swipe_info TEXT NOT NULL,
                                    PRIMARY KEY (chat_id, id));
             CREATE TABLE cocreator_sessions (id TEXT PRIMARY KEY, title TEXT, created INTEGER,
                                              modified INTEGER NOT NULL);
             CREATE TABLE cocreator_messages (session_id TEXT NOT NULL, id TEXT NOT NULL,
                                              position INTEGER NOT NULL, is_user INTEGER NOT NULL,
                                              swipe_id INTEGER, swipes TEXT NOT NULL,
                                              swipe_info TEXT NOT NULL,
                                              PRIMARY KEY (session_id, id));
             ",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chats VALUES (?1,'Ayla.png','First contact',1,?2,0,'{}')",
            rusqlite::params![chat, modified],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages VALUES (?1,'msg-1',1,'Ayla',0,0,0,'[\"hi\"]',?2)",
            rusqlite::params![chat, swipes],
        )
        .unwrap();
        drop(conn);

        let dir = home.join(".wackchatter");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("library.json"),
            format!(
                "{{\"version\":1,\"dataDir\":{:?}}}",
                lib.display().to_string()
            ),
        )
        .unwrap();
        db
    }

    const SWIPE: &str = r#"[{"send_date":"2026-08-31T10:00:00.000Z","gen_started":"2026-08-31T10:00:00.000Z","gen_finished":"2026-08-31T10:00:05.000Z","extra":{"api":"openrouter","model":"anthropic/claude-sonnet-5","token_count":612,"prompt_tokens":8421,"usage_reported":true,"generation_id":"gen-1"}}]"#;

    /// What almost all stored history actually looks like: an estimated completion count,
    /// no prompt side, and no generation id, because none of that existed when it was
    /// written. Its identity comes from its place in the transcript instead.
    const LEGACY_SWIPE: &str = r#"[{"send_date":"2026-01-02T03:04:05.000Z","extra":{"api":"openrouter","model":"sao10k/l3-euryale-70b","token_count":420}}]"#;

    #[test]
    fn backfills_transcripts_through_the_library_pointer() {
        let home = test_home("wc-db");
        let store = test_store("wc-db");
        seed_library(&home, SWIPE);

        assert_eq!(collect(&store, &home).unwrap(), 1);
        let (id, session, project, model, provider, input, output, dur, est): (
            String,
            String,
            String,
            String,
            String,
            i64,
            i64,
            i64,
            i64,
        ) = store
            .read_conn()
            .query_row(
                "SELECT source_event_id, session_id, project, model, provider,
                        input_tokens, output_tokens, COALESCE(duration_ms,0), estimated
                 FROM usage_event",
                [],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                        r.get(8)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(id, "gen-1");
        assert_eq!(session, "chat-a");
        assert_eq!(project, "WackChatter · Ayla");
        assert_eq!(
            (model.as_str(), provider.as_str()),
            ("claude-sonnet-5", "openrouter")
        );
        assert_eq!((input, output, dur, est), (8421, 612, 5000, 0));
    }

    #[test]
    fn a_swipe_and_its_live_log_twin_are_one_event() {
        // The whole reason the generation id exists: the same reply reaching the store
        // down both paths must not be counted twice.
        let home = test_home("wc-dedup");
        let store = test_store("wc-dedup");
        seed_library(&home, SWIPE);
        seed_log(&home, &format!("{LINE}\n"));

        assert_eq!(collect(&store, &home).unwrap(), 2); // one insert, one upsert
        let rows: i64 = store
            .read_conn()
            .query_row("SELECT COUNT(*) FROM usage_event", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1);
    }

    #[test]
    fn pre_id_history_falls_back_to_its_place_in_the_transcript() {
        let home = test_home("wc-legacy");
        let store = test_store("wc-legacy");
        // What every swipe written before 2026-08 looks like: an estimated completion
        // count, no prompt side, no id, no reported flag.
        seed_library(&home, LEGACY_SWIPE);

        assert_eq!(collect(&store, &home).unwrap(), 1);
        let (id, input, output, est, cost): (String, i64, i64, i64, Option<f64>) = store
            .read_conn()
            .query_row(
                "SELECT source_event_id, input_tokens, output_tokens, estimated, cost_usd
                 FROM usage_event",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap();
        assert_eq!(id, "Library:wcd:chat-a:msg-1:0");
        assert_eq!((input, output, est), (0, 420, 1));
        // A roleplay finetune is not in the bundled price list, and an unpriced event has
        // no cost rather than a zero one.
        assert_eq!(cost, None);
    }

    /// The cross-repo contract.
    ///
    /// `fixtures/wackchatter_sample.jsonl` is captured verbatim from WackChatter's own
    /// POST /api/usage, not hand-written — so if either side changes the wire format
    /// without the other, this is what says so.
    #[test]
    fn reads_wackchatters_own_output() {
        let home = test_home("wc-fixture");
        let store = test_store("wc-fixture");
        let dir = home.join(".wackchatter");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(fixture("wackchatter_sample.jsonl"), dir.join("usage.jsonl")).unwrap();

        assert_eq!(collect(&store, &home).unwrap(), 4);

        // One row per generation, across all four features, with the model strings
        // normalised for pricing and the background run marked as such.
        let mut rows: Vec<(String, String, i64, i64, i64, i64)> = store
            .read_conn()
            .prepare(
                "SELECT source_event_id, COALESCE(model,''), input_tokens, output_tokens,
                        estimated, is_subagent
                 FROM usage_event ORDER BY ts",
            )
            .unwrap()
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            })
            .unwrap()
            .flatten()
            .collect();
        rows.sort();
        assert_eq!(
            rows,
            vec![
                // A reply with reported usage: the cached half is already subtracted.
                (
                    "gen-smoke-1".into(),
                    "claude-sonnet-5".into(),
                    8421,
                    612,
                    0,
                    0
                ),
                // A rolling summary: nothing reported, so estimated, and not a top-level turn.
                ("gen-smoke-2".into(), "glm-5.2".into(), 0, 180, 1, 1),
                // An Arena round: a person drove it, so not a background run.
                (
                    "gen-smoke-3".into(),
                    "l3-euryale-70b".into(),
                    2200,
                    410,
                    0,
                    0
                ),
                // Stopped mid-reply. It still generated, so it still counts.
                ("gen-smoke-4".into(), "claude-sonnet-5".into(), 0, 57, 1, 0),
            ]
        );

        // Models the bundled price list already knows cost what they cost; a roleplay
        // finetune it has never heard of gets no cost rather than a zero one, and still
        // contributes its tokens. This is what "tokens now, pricing later" looks like.
        let unpriced: Vec<String> = store
            .read_conn()
            .prepare("SELECT model FROM usage_event WHERE cost_usd IS NULL")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(unpriced, vec!["l3-euryale-70b"]);

        assert_eq!(collect(&store, &home).unwrap(), 0);
    }

    /// Point the pointer at a second, older library and its history must still land.
    ///
    /// This is a real bug that shipped: one cursor shared by every library meant the
    /// newer one's timestamp hid the older one entirely, and permanently — nothing in a
    /// skipped library will ever be modified later than a cursor that has already passed
    /// it. Two checkouts on one machine is the ordinary way to hit it; moving a library
    /// to a drive holding an older copy is the other.
    #[test]
    fn a_second_older_library_is_not_swallowed_by_the_first() {
        let home = test_home("wc-two-libs");
        let store = test_store("wc-two-libs");
        let pointer = home.join(".wackchatter/library.json");

        // The library in daily use, touched this morning.
        let recent =
            seed_named_library(&home, "Recent", "chat-recent", LEGACY_SWIPE, 1756634405000);
        collect(&store, &home).unwrap();

        // A second library whose newest chat is a full day older.
        let older = seed_named_library(&home, "Older", "chat-older", LEGACY_SWIPE, 1756548005000);
        assert_ne!(recent, older);
        std::fs::write(
            &pointer,
            format!(
                "{{\"version\":1,\"dataDir\":{:?}}}",
                older.parent().unwrap().display().to_string()
            ),
        )
        .unwrap();

        collect(&store, &home).unwrap();
        assert_eq!(
            sessions(&store),
            vec!["chat-older", "chat-recent"],
            "the older library was swallowed by the newer one's cursor"
        );
    }

    #[test]
    fn a_library_keeps_its_cursor_when_the_pointer_moves_away_and_back() {
        let home = test_home("wc-lib-cursor");
        let store = test_store("wc-lib-cursor");
        let a = seed_named_library(&home, "A", "chat-a", LEGACY_SWIPE, 1756634405000);
        let b = seed_named_library(&home, "B", "chat-b", LEGACY_SWIPE, 1756548005000);
        aim_pointer(&home, a.parent().unwrap());
        collect(&store, &home).unwrap();
        aim_pointer(&home, b.parent().unwrap());
        collect(&store, &home).unwrap();
        assert_eq!(sessions(&store), vec!["chat-a", "chat-b"]);

        // Each library's cursor is its own, so a settled sync re-reads only the overlap
        // window and never the whole transcript again.
        let touched = collect(&store, &home).unwrap();
        assert!(
            touched <= 2,
            "a settled sync rescanned more than the overlap: {touched}"
        );
    }

    /// Add a chat to a library that already exists, without touching the pointer.
    fn add_chat(data_dir: &Path, chat: &str, swipes: &str, modified: i64) {
        let conn = rusqlite::Connection::open(data_dir.join("chats.db")).unwrap();
        conn.execute(
            "INSERT INTO chats VALUES (?1,'Brooke.png','Later',1,?2,0,'{}')",
            rusqlite::params![chat, modified],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages VALUES (?1,'msg-1',1,'Brooke',0,0,0,'[\"hi\"]',?2)",
            rusqlite::params![chat, swipes],
        )
        .unwrap();
    }

    fn sessions(store: &Store) -> Vec<String> {
        store
            .read_conn()
            .prepare("SELECT DISTINCT session_id FROM usage_event ORDER BY session_id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .flatten()
            .collect()
    }

    fn aim_pointer(home: &Path, data_dir: &Path) {
        std::fs::write(
            home.join(".wackchatter/library.json"),
            format!(
                "{{\"version\":1,\"dataDir\":{:?}}}",
                data_dir.display().to_string()
            ),
        )
        .unwrap();
    }

    /// A library goes on being read after the pointer has moved off it.
    ///
    /// The pointer names whichever install booted last. Two checkouts on one machine is
    /// an ordinary setup, and reading only the current one means the other silently stops
    /// being tracked the moment you launch its sibling.
    #[test]
    fn every_library_seen_stays_current_not_just_the_pointed_one() {
        let home = test_home("wc-all-libs");
        let store = test_store("wc-all-libs");
        let a = seed_named_library(&home, "A", "chat-a", LEGACY_SWIPE, 1756634405000);
        let b = seed_named_library(&home, "B", "chat-b", LEGACY_SWIPE, 1756548005000);
        let (a_dir, b_dir) = (
            a.parent().unwrap().to_path_buf(),
            b.parent().unwrap().to_path_buf(),
        );

        // Both get registered as the pointer visits them.
        aim_pointer(&home, &a_dir);
        collect(&store, &home).unwrap();
        aim_pointer(&home, &b_dir);
        collect(&store, &home).unwrap();
        assert_eq!(sessions(&store), vec!["chat-a", "chat-b"]);

        // New work in A while the pointer still names B — the case that was broken.
        add_chat(&a_dir, "chat-a2", LEGACY_SWIPE, 1756720805000);
        collect(&store, &home).unwrap();
        assert!(
            sessions(&store).contains(&"chat-a2".to_string()),
            "A stopped being read"
        );

        // And new work in B is still picked up too.
        add_chat(&b_dir, "chat-b2", LEGACY_SWIPE, 1756720805000);
        collect(&store, &home).unwrap();
        assert_eq!(
            sessions(&store),
            vec!["chat-a", "chat-a2", "chat-b", "chat-b2"]
        );
    }

    /// An unplugged drive is not a deleted library.
    #[test]
    fn a_library_that_goes_missing_is_skipped_and_picked_up_again() {
        let home = test_home("wc-missing-lib");
        let store = test_store("wc-missing-lib");
        let a = seed_named_library(&home, "A", "chat-a", LEGACY_SWIPE, 1756634405000);
        let a_dir = a.parent().unwrap().to_path_buf();
        collect(&store, &home).unwrap();
        assert_eq!(sessions(&store), vec!["chat-a"]);

        // The drive comes out. The sync must not fail, and the library must not be
        // forgotten — being unreachable now says nothing about being gone for good.
        let stashed = home.join("stashed.db");
        std::fs::rename(&a, &stashed).unwrap();
        assert_eq!(collect(&store, &home).unwrap(), 0);
        assert_eq!(
            sessions(&store),
            vec!["chat-a"],
            "history must survive the absence"
        );

        // It comes back with new work on it, and is read without being re-pointed at.
        std::fs::rename(&stashed, &a).unwrap();
        add_chat(&a_dir, "chat-a2", LEGACY_SWIPE, 1756720805000);
        collect(&store, &home).unwrap();
        assert_eq!(sessions(&store), vec!["chat-a", "chat-a2"]);
    }

    #[test]
    fn a_missing_or_broken_pointer_leaves_the_log_working() {
        let home = test_home("wc-nopointer");
        let store = test_store("wc-nopointer");
        seed_log(&home, &format!("{LINE}\n"));
        std::fs::write(home.join(".wackchatter/library.json"), "{ not json").unwrap();

        assert_eq!(collect(&store, &home).unwrap(), 1);
    }
    #[test]
    fn durable_segments_replay_without_losing_ledger_quality() {
        let home = test_home("wc-v2-replay");
        let store = test_store("wc-v2-replay-store");
        let dir = home.join(".wackchatter/usage/v2/2026-10");
        std::fs::create_dir_all(&dir).unwrap();
        let mut record: serde_json::Value = serde_json::from_str(LINE).unwrap();
        record["v"] = serde_json::json!(2);
        record["library_id"] = serde_json::json!("lib");
        record["provider_name"] = serde_json::json!("My connection");
        record["outcome"] = serde_json::json!("completed");
        let path = dir.join("writer.jsonl");
        std::fs::write(&path, record.to_string()).unwrap();
        assert_eq!(collect(&store, &home).unwrap(), 0);
        std::fs::write(&path, format!("{}\n", record)).unwrap();
        assert_eq!(collect(&store, &home).unwrap(), 1);
        let mut transcript = parse_log_line(LINE).unwrap();
        transcript.cache_read_tokens = 0;
        transcript.reasoning_tokens = None;
        transcript.ttft_ms = None;
        transcript.provider_name = None;
        transcript.purpose = None;
        let context = WackContext {
            rank: 1,
            library_id: Some("lib".into()),
            character_id: Some("Ayla.png".into()),
            legacy_id: None,
        };
        store
            .insert_wackchatter(&[transcript.clone()], &[context])
            .unwrap();
        let detail = crate::wackchatter::usage(&store, Some("lib"), 0, None).unwrap();
        assert_eq!(detail.totals.events, 1);
        assert_eq!(detail.totals.cache_read_tokens, 4096);
        assert_eq!(detail.providers[0].name, "My connection");
        transcript.session_id = Some("branch".into());
        store
            .insert_wackchatter(
                &[transcript],
                &[WackContext {
                    rank: 1,
                    library_id: Some("lib".into()),
                    character_id: Some("Ayla.png".into()),
                    legacy_id: None,
                }],
            )
            .unwrap();
        assert_eq!(
            crate::wackchatter::usage(&store, Some("lib"), 0, Some("branch"))
                .unwrap()
                .calls
                .len(),
            1
        );
        assert_eq!(
            crate::wackchatter::usage(&store, Some("lib"), 0, None)
                .unwrap()
                .totals
                .events,
            1
        );
        // Replacing with a longer file must replay its prefix, not skip the first record.
        record["id"] = serde_json::json!("replacement-generation");
        std::fs::write(&path, format!("{}\n{}\n", record, record)).unwrap();
        collect(&store, &home).unwrap();
        assert_eq!(
            crate::wackchatter::usage(&store, None, 0, None)
                .unwrap()
                .totals
                .events,
            2
        );
    }
    #[test]
    fn measured_transcript_beats_estimated_ledger_in_either_order() {
        for reverse in [false, true] {
            let store = test_store(if reverse {
                "wc-quality-reverse"
            } else {
                "wc-quality-forward"
            });
            let measured = parse_log_line(LINE).unwrap();
            let mut estimated = measured.clone();
            estimated.estimated = true;
            estimated.output_tokens = 99999;
            let items = if reverse {
                vec![(measured, 1), (estimated, 2)]
            } else {
                vec![(estimated, 2), (measured, 1)]
            };
            for (event, rank) in items {
                store
                    .insert_wackchatter(
                        &[event],
                        &[WackContext {
                            rank,
                            library_id: Some("lib".into()),
                            ..Default::default()
                        }],
                    )
                    .unwrap();
            }
            let d = crate::wackchatter::usage(&store, Some("lib"), 0, None).unwrap();
            assert_eq!(d.totals.estimated_tokens, 0);
            assert_eq!(d.totals.output_tokens, 612);
            assert_eq!(d.totals.measured_events, 1);
        }
    }
    #[test]
    fn source_wide_usage_keeps_unattributed_and_unpriced_history() {
        let store = test_store("wc-unattributed");
        let mut e = parse_log_line(LINE).unwrap();
        e.ts = 1;
        e.estimated = true;
        e.session_id = None;
        store
            .insert_wackchatter(
                &[e],
                &[WackContext {
                    rank: 2,
                    ..Default::default()
                }],
            )
            .unwrap();
        store.reprice_all().unwrap();
        let cost: Option<f64> = store
            .read_conn()
            .query_row("SELECT cost_usd FROM usage_event", [], |r| r.get(0))
            .unwrap();
        assert_eq!(cost, None, "repricing must never price estimated tokens");
        let d = crate::wackchatter::usage(&store, None, 0, Some("unknown")).unwrap();
        assert_eq!(d.totals.events, 1);
        assert_eq!(d.totals.cost_usd, None);
        assert_eq!(d.totals.measured_events, 0);
        assert_eq!(d.calls.len(), 1);
        assert_eq!(
            crate::wackchatter::usage(&store, None, 90, None)
                .unwrap()
                .totals
                .events,
            0
        );
        assert_eq!(
            crate::wackchatter::usage(&store, Some("lib"), 0, None)
                .unwrap()
                .totals
                .events,
            0
        );
    }
    #[test]
    fn copied_generations_keep_original_attribution_and_scoped_conversations() {
        let store = test_store("wc-copied-context");
        let original = parse_log_line(LINE).unwrap();
        let mut copied = original.clone();
        copied.session_id = Some("copied-chat".into());
        for (event, rank, library, character) in [
            (original, 2, "original-library", "Z.png"),
            (copied, 1, "copied-library", "A.png"),
        ] {
            store
                .insert_wackchatter(
                    &[event],
                    &[WackContext {
                        rank,
                        library_id: Some(library.into()),
                        character_id: Some(character.into()),
                        legacy_id: None,
                    }],
                )
                .unwrap();
        }
        let all = crate::wackchatter::usage(&store, None, 0, None).unwrap();
        assert_eq!(all.totals.events, 1);
        assert_eq!(all.characters[0].name, "original-library · Z.png");
        let copied = crate::wackchatter::usage(&store, Some("copied-library"), 0, None).unwrap();
        assert_eq!(copied.conversations[0].name, "copied-chat");
        assert_eq!(copied.characters[0].name, "A.png");
        assert_eq!(
            crate::wackchatter::usage(&store, Some("copied-library"), 0, Some("copied-chat"))
                .unwrap()
                .calls
                .len(),
            1
        );
    }
}
