use crate::collectors::{clean_model, parse_ts_ms, read_tail, sorted_glob};
use crate::models::{Source, UsageEvent};
use crate::store::{open_readonly, Store};
use std::path::{Path, PathBuf};

/// WackChatter — a roleplay chat frontend for OpenAI-compatible providers.
///
/// Two readers, because the app records usage in two places for two reasons:
///
///   * `~/.wackchatter/usage.jsonl` — one line per generation, written live once the user
///     turns the log on. The only source for the generations that never become a stored
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
    Ok(collect_log(store, &dir)? + collect_db(store, &dir).unwrap_or(0))
}

// ---------------------------------------------------------------------------
// The live log
// ---------------------------------------------------------------------------

/// Only `usage.jsonl` — a rotated `usage.jsonl.1` is deliberately not globbed. Its records
/// are either already stored or old enough that `chats.db` is the better source for them.
fn collect_log(store: &Store, dir: &Path) -> Result<usize, String> {
    let files = sorted_glob(&format!("{}/usage.jsonl", dir.display()))?;
    let mut processed = 0usize;
    for path in files {
        let key = path.display().to_string();
        let offset = store.get_offset("wackchatter", &key).unwrap_or(0);
        let (tail, new_offset) = read_tail(&path, offset).map_err(|e| format!("read {key}: {e}"))?;
        if tail.is_empty() {
            if new_offset != offset {
                store.set_offset("wackchatter", &key, new_offset);
            }
            continue;
        }
        let events: Vec<UsageEvent> = tail.lines().filter_map(parse_log_line).collect();
        processed += store.insert_events(&events).map_err(|e| format!("wackchatter insert: {e}"))?;
        store.set_offset("wackchatter", &key, new_offset);
    }
    Ok(processed)
}

fn parse_log_line(line: &str) -> Option<UsageEvent> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    // Version gate rather than best-effort parsing: a future writer that changes what a
    // field means must not be read as if it still meant the old thing.
    if v.get("v").and_then(|x| x.as_i64()) != Some(1) {
        return None;
    }
    let id = v.get("id").and_then(|x| x.as_str()).filter(|s| !s.is_empty())?;
    let ts = v.get("ts").and_then(|x| x.as_str()).and_then(parse_ts_ms)?;
    let num = |k: &str| v.get(k).and_then(|x| x.as_i64()).unwrap_or(0);
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
    })
}

/// Work the app does for itself rather than a reply the user asked for.
///
/// The same idea as a harness sidechain: in service of the conversation, not part of it.
/// Co-Creator and Arena are excluded — a person is driving both of those.
fn is_background(feature: &str) -> bool {
    matches!(feature, "summary" | "memory" | "persona")
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

/// Resolve the library from the pointer WackChatter publishes on every boot.
fn library_db(dir: &Path) -> Option<PathBuf> {
    let raw = std::fs::read_to_string(dir.join("library.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let data_dir = v.get("dataDir").and_then(|x| x.as_str())?;
    let db = PathBuf::from(data_dir).join("chats.db");
    db.exists().then_some(db)
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
                {owner}
         FROM {table} m, json_each(m.swipe_info) s, {parent} p
         WHERE p.id = m.{parent_key} AND m.is_user = 0 AND m.position > 0
           AND p.modified >= ?1 {extra_where}",
        parent_key = if parent == "chats" { "chat_id" } else { "session_id" },
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
);

fn collect_db(store: &Store, dir: &Path) -> Option<usize> {
    let db = library_db(dir)?;
    let conn = open_readonly(&db).ok()?;

    let mut processed = 0usize;
    // Two watermarks, because the two tables advance independently: a week of Co-Creator
    // work must not be skipped because a chat was touched more recently.
    for (table, parent, owner, prefix, key) in [
        ("messages", "chats", "p.character_id", "wcd", "wackchatter:chats"),
        ("cocreator_messages", "cocreator_sessions", "NULL", "wcc", "wackchatter:cocreator"),
    ] {
        // Overlapping the watermark covers the window between a swipe being written and
        // its chat's `modified` settling, the same guard opencode.rs uses.
        let watermark = store.get_watermark(key).saturating_sub(OVERLAP_MS);
        // Co-Creator rows carry no is_system column — the table has two speakers and no
        // hiding — so that filter belongs only to the chat transcripts.
        let extra = if table == "messages" { "AND m.is_system = 0" } else { "" };

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
                r.get(12)?,
            ))
        }) else {
            continue;
        };

        let mut events = Vec::new();
        let mut max_ts = 0i64;
        for row in rows.flatten() {
            let Some(ev) = from_swipe(row, prefix) else { continue };
            max_ts = max_ts.max(ev.ts);
            events.push(ev);
        }
        processed += store.insert_events(&events).ok()?;
        if max_ts > 0 {
            store.set_watermark(key, max_ts);
        }
    }
    Some(processed)
}

fn from_swipe(row: SwipeRow, prefix: &str) -> Option<UsageEvent> {
    let (parent, message, swipe, gen_id, model, api, out, prompt, reported, started, finished, sent, character) =
        row;

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

    let duration_ms = match (started.as_deref().and_then(parse_ts_ms), finished.as_deref().and_then(parse_ts_ms)) {
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
        let (id, input, output, cr, reasoning, ttft, est): (
            String,
            i64,
            i64,
            i64,
            i64,
            i64,
            i64,
        ) = store
            .conn()
            .query_row(
                "SELECT source_event_id, input_tokens, output_tokens, cache_read_tokens,
                        COALESCE(reasoning_tokens,0), COALESCE(ttft_ms,0), estimated
                 FROM usage_event",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
            )
            .unwrap();
        assert_eq!((id.as_str(), input, output, cr, reasoning, ttft, est), ("gen-1", 8421, 612, 4096, 200, 640, 0));

        // The OpenRouter vendor prefix is stripped so pricing and families can match.
        let (model, project, session): (String, String, String) = store
            .conn()
            .query_row("SELECT model, project, session_id FROM usage_event", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
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
        let v2 = LINE.replace("\"v\":1", "\"v\":2").replace("\"gen-1\"", "\"gen-v2\"");
        // The last line has no newline: a half-written tail must be retried, not parsed.
        let partial = LINE.replace("\"gen-1\"", "\"gen-partial\"");
        seed_log(&home, &format!("{LINE}\n{zero}\n{v2}\nnot json\n{partial}"));

        assert_eq!(collect(&store, &home).unwrap(), 1);
        let ids: Vec<String> = store
            .conn()
            .prepare("SELECT source_event_id FROM usage_event")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(ids, vec!["gen-1"]);

        // Completing the partial line is what makes it readable.
        seed_log(&home, &format!("{LINE}\n{zero}\n{v2}\nnot json\n{partial}\n"));
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
        assert_eq!(parse_log_line(&line).unwrap().project.as_deref(), Some("WackChatter"));
    }

    /// A minimal WackChatter library: the two transcript tables and a pointer to them.
    fn seed_library(home: &Path, swipes: &str) -> PathBuf {
        let lib = home.join("Library");
        std::fs::create_dir_all(&lib).unwrap();
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
             INSERT INTO chats VALUES ('chat-a','Ayla.png','First contact',1,1756634405000,0,'{}');",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages VALUES ('chat-a','msg-1',1,'Ayla',0,0,0,'[\"hi\"]',?1)",
            [swipes],
        )
        .unwrap();
        drop(conn);

        let dir = home.join(".wackchatter");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("library.json"),
            format!("{{\"version\":1,\"dataDir\":{:?}}}", lib.display().to_string()),
        )
        .unwrap();
        db
    }

    const SWIPE: &str = r#"[{"send_date":"2026-08-31T10:00:00.000Z","gen_started":"2026-08-31T10:00:00.000Z","gen_finished":"2026-08-31T10:00:05.000Z","extra":{"api":"openrouter","model":"anthropic/claude-sonnet-5","token_count":612,"prompt_tokens":8421,"usage_reported":true,"generation_id":"gen-1"}}]"#;

    #[test]
    fn backfills_transcripts_through_the_library_pointer() {
        let home = test_home("wc-db");
        let store = test_store("wc-db");
        seed_library(&home, SWIPE);

        assert_eq!(collect(&store, &home).unwrap(), 1);
        let (id, session, project, model, provider, input, output, dur, est): (
            String, String, String, String, String, i64, i64, i64, i64,
        ) = store
            .conn()
            .query_row(
                "SELECT source_event_id, session_id, project, model, provider,
                        input_tokens, output_tokens, COALESCE(duration_ms,0), estimated
                 FROM usage_event",
                [],
                |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?,
                        r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?))
                },
            )
            .unwrap();
        assert_eq!(id, "gen-1");
        assert_eq!(session, "chat-a");
        assert_eq!(project, "WackChatter · Ayla");
        assert_eq!((model.as_str(), provider.as_str()), ("claude-sonnet-5", "openrouter"));
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
        let rows: i64 =
            store.conn().query_row("SELECT COUNT(*) FROM usage_event", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 1);
    }

    #[test]
    fn pre_id_history_falls_back_to_its_place_in_the_transcript() {
        let home = test_home("wc-legacy");
        let store = test_store("wc-legacy");
        // What every swipe written before 2026-08 looks like: an estimated completion
        // count, no prompt side, no id, no reported flag.
        seed_library(
            &home,
            r#"[{"send_date":"2026-01-02T03:04:05.000Z","extra":{"api":"openrouter","model":"sao10k/l3-euryale-70b","token_count":420}}]"#,
        );

        assert_eq!(collect(&store, &home).unwrap(), 1);
        let (id, input, output, est, cost): (String, i64, i64, i64, Option<f64>) = store
            .conn()
            .query_row(
                "SELECT source_event_id, input_tokens, output_tokens, estimated, cost_usd
                 FROM usage_event",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap();
        assert_eq!(id, "wcd:chat-a:msg-1:0");
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
            .conn()
            .prepare(
                "SELECT source_event_id, COALESCE(model,''), input_tokens, output_tokens,
                        estimated, is_subagent
                 FROM usage_event ORDER BY ts",
            )
            .unwrap()
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))
            })
            .unwrap()
            .flatten()
            .collect();
        rows.sort();
        assert_eq!(
            rows,
            vec![
                // A reply with reported usage: the cached half is already subtracted.
                ("gen-smoke-1".into(), "claude-sonnet-5".into(), 8421, 612, 0, 0),
                // A rolling summary: nothing reported, so estimated, and not a top-level turn.
                ("gen-smoke-2".into(), "glm-5.2".into(), 0, 180, 1, 1),
                // An Arena round: a person drove it, so not a background run.
                ("gen-smoke-3".into(), "l3-euryale-70b".into(), 2200, 410, 0, 0),
                // Stopped mid-reply. It still generated, so it still counts.
                ("gen-smoke-4".into(), "claude-sonnet-5".into(), 0, 57, 1, 0),
            ]
        );

        // Models the bundled price list already knows cost what they cost; a roleplay
        // finetune it has never heard of gets no cost rather than a zero one, and still
        // contributes its tokens. This is what "tokens now, pricing later" looks like.
        let unpriced: Vec<String> = store
            .conn()
            .prepare("SELECT model FROM usage_event WHERE cost_usd IS NULL")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(unpriced, vec!["l3-euryale-70b"]);

        assert_eq!(collect(&store, &home).unwrap(), 0);
    }

    #[test]
    fn a_missing_or_broken_pointer_leaves_the_log_working() {
        let home = test_home("wc-nopointer");
        let store = test_store("wc-nopointer");
        seed_log(&home, &format!("{LINE}\n"));
        std::fs::write(home.join(".wackchatter/library.json"), "{ not json").unwrap();

        assert_eq!(collect(&store, &home).unwrap(), 1);
    }
}
