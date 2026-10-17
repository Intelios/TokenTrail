//! WackChatter's current library, read-only. No transcript text crosses the IPC boundary.
use crate::{
    collectors::wackchatter::{library_key, pointed_library},
    store::{open_readonly, Store},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

type Result<T> = std::result::Result<T, String>;
#[derive(Serialize, Deserialize, Clone)]
pub struct Library {
    pub id: String,
    pub name: String,
    pub available: bool,
    pub current: bool,
}
#[derive(Serialize, Deserialize, Default)]
pub struct Totals {
    pub chats: i64,
    pub messages: i64,
    pub user_messages: i64,
    pub replies: i64,
    pub swipes: i64,
    pub measured_tokens: i64,
    pub estimated_tokens: i64,
    pub active_minutes: i64,
}
#[derive(Serialize, Deserialize, Default)]
pub struct Habits {
    pub rerolls: i64,
    pub rerolled_replies: i64,
    pub eligible_replies: i64,
    pub avg_user_chars: f64,
    pub avg_reply_chars: f64,
    pub longest_reply_chars: i64,
    pub reasoning_swipes: i64,
    pub timed_swipes: i64,
    pub branched_chats: i64,
    pub deleted_chats: i64,
}
#[derive(Serialize, Deserialize, Clone)]
pub struct Chat {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub character_id: Option<String>,
    pub messages: i64,
    pub replies: i64,
    pub measured_tokens: i64,
    pub estimated_tokens: i64,
    pub created: i64,
    pub modified: i64,
}
#[derive(Serialize, Deserialize)]
pub struct Cast {
    pub character_id: String,
    pub name: String,
    pub chats: i64,
    pub replies: i64,
    pub measured_tokens: i64,
    pub estimated_tokens: i64,
    pub first_chat: i64,
    pub last_chat: i64,
}
#[derive(Serialize, Deserialize)]
pub struct Model {
    pub model: String,
    pub provider: String,
    pub swipes: i64,
    pub measured_tokens: i64,
    pub estimated_tokens: i64,
    pub timed_swipes: i64,
    pub total_ms: f64,
    pub measured_ms: f64,
    pub measured_timed_tokens: i64,
    pub estimated_ms: f64,
    pub estimated_timed_tokens: i64,
}
#[derive(Serialize, Deserialize)]
pub struct Persona {
    pub persona_id: Option<String>,
    pub name: String,
    pub chats: i64,
    pub messages: i64,
}
#[derive(Serialize)]
pub struct Stats {
    pub totals: Totals,
    pub habits: Habits,
    pub cast: Vec<Cast>,
    pub chats: Vec<Chat>,
    pub groups: Vec<Chat>,
    pub models: Vec<Model>,
    pub personas: Vec<Persona>,
    pub hours: Vec<(i64, i64)>,
    pub co_creator_sessions: i64,
    pub first_chat: Option<i64>,
    pub last_chat: Option<i64>,
    pub name: String,
}

fn paths(store: &Store, home: &Path) -> Vec<PathBuf> {
    let current = pointed_library(&home.join(".wackchatter"));
    let mut paths: Vec<PathBuf> = store
        .known_paths("wackchatter:library")
        .into_iter()
        .map(PathBuf::from)
        .collect();
    if let Some(p) = current {
        if !paths.contains(&p) {
            paths.push(p);
        }
    }
    paths
}
pub fn libraries(store: &Store, home: &Path) -> Vec<Library> {
    let current = pointed_library(&home.join(".wackchatter"));
    let mut by_id = BTreeMap::new();
    for path in paths(store, home) {
        let id = if path.join("chats.db").exists() {
            library_key(&path)
        } else {
            store
                .wackchatter_library_id(&path.to_string_lossy())
                .unwrap_or_else(|| library_key(&path))
        };
        let item = Library {
            id: id.clone(),
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into(),
            available: path.join("chats.db").exists(),
            current: current.as_ref() == Some(&path),
        };
        if !by_id.contains_key(&id) || item.available {
            by_id.insert(id, item);
        }
    }
    by_id.into_values().collect()
}
fn root(store: &Store, home: &Path, id: &str) -> Result<PathBuf> {
    paths(store,home).into_iter().filter(|p|library_key(p)==id).find(|p|p.join("chats.db").exists()).ok_or_else(||"This WackChatter library is unavailable. Reconnect its drive or start WackChatter with this library.".into())
}
/// SQL resolving `model` at epoch-ms `ts` through `aliases`, for databases
/// that cannot join TokenTrail's `model_alias` (the counterpart of
/// `store::MODEL_ALIAS_JOIN`). Values are user-chosen names, so they are
/// quoted as SQL literals rather than trusted.
fn alias_case_sql(aliases: &[crate::models::ModelAlias], model: &str, ts: &str) -> String {
    if aliases.is_empty() {
        return model.to_string();
    }
    let q = |v: &str| format!("'{}'", v.replace('\'', "''"));
    let arms: String = aliases
        .iter()
        .map(|a| {
            format!(
                " WHEN m.v={} AND t.v>={} AND t.v<{} THEN {}",
                q(&a.alias),
                a.from_ts,
                a.until_ts,
                q(&a.canonical)
            )
        })
        .collect();
    format!("(SELECT CASE{arms} ELSE m.v END FROM (SELECT {model} AS v) m, (SELECT {ts} AS v) t)")
}

fn rows<T: serde::de::DeserializeOwned>(
    db: &Connection,
    sql: &str,
    args: &[&dyn rusqlite::ToSql],
) -> Result<Vec<T>> {
    let mut stmt = db
        .prepare(sql)
        .map_err(|e| format!("WackChatter query: {e}"))?;
    let names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let mapped = stmt
        .query_map(args, |r| {
            let mut obj = serde_json::Map::new();
            for (i, name) in names.iter().enumerate() {
                use rusqlite::types::ValueRef;
                let v = match r.get_ref(i)? {
                    ValueRef::Null => Value::Null,
                    ValueRef::Integer(n) => json!(n),
                    ValueRef::Real(n) => json!(n),
                    ValueRef::Text(s) => json!(String::from_utf8_lossy(s)),
                    ValueRef::Blob(_) => Value::Null,
                };
                obj.insert(name.clone(), v);
            }
            Ok(Value::Object(obj))
        })
        .map_err(|e| e.to_string())?;
    mapped
        .map(|r| serde_json::from_value(r.map_err(|e| e.to_string())?).map_err(|e| e.to_string()))
        .collect()
}
fn scalar(db: &Connection, sql: &str) -> i64 {
    db.query_row(sql, [], |r| r.get(0)).unwrap_or(0)
}
fn has(db: &Connection, table: &str, col: &str) -> bool {
    db.prepare("SELECT 1 FROM pragma_table_info(?1) WHERE name=?2")
        .and_then(|mut s| s.exists(params![table, col]))
        .unwrap_or(false)
}
fn names(root: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for p in cards(&root.join("characters")) {
        if let Some(id) = p.file_name().and_then(|s| s.to_str()) {
            out.insert(
                id.into(),
                card_name(&p).unwrap_or_else(|| id.trim_end_matches(".png").into()),
            );
        }
    }
    out
}
fn cards(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            let Ok(t) = e.file_type() else { continue };
            if t.is_dir() {
                out.extend(cards(&p));
            } else if t.is_file() && p.extension().is_some_and(|s| s.eq_ignore_ascii_case("png")) {
                out.push(p);
            }
        }
    }
    out
}
fn card_name(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return None;
    }
    let mut pos = 8;
    let mut v2 = None;
    let mut v3 = None;
    while pos + 12 <= bytes.len() {
        let n = u32::from_be_bytes(bytes[pos..pos + 4].try_into().ok()?) as usize;
        let end = pos.checked_add(n + 12)?;
        if end > bytes.len() {
            break;
        }
        if &bytes[pos + 4..pos + 8] == b"tEXt" {
            let payload = &bytes[pos + 8..pos + 8 + n];
            if let Some(z) = payload.iter().position(|b| *b == 0) {
                let key = String::from_utf8_lossy(&payload[..z]).to_lowercase();
                if key == "chara" || key == "ccv3" {
                    let raw = STANDARD.decode(&payload[z + 1..]).ok();
                    if key == "ccv3" {
                        v3 = raw;
                    } else {
                        v2 = raw;
                    }
                }
            }
        }
        pos = end;
    }
    let data: Value = serde_json::from_slice(&v3.or(v2)?).ok()?;
    data.pointer("/data/name")
        .or_else(|| data.get("name"))
        .and_then(Value::as_str)
        .map(String::from)
}
fn persona_name(root: &Path, id: Option<&str>) -> String {
    let Some(id) = id else {
        return "Unknown persona (legacy)".into();
    };
    if id.is_empty() {
        return "No persona".into();
    }
    let Ok(p) = safe_file(root, &root.join("personas"), &format!("{id}.json")) else {
        return "Deleted persona".into();
    };
    let v: Value = fs::read(p)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null);
    let name = v["name"].as_str().unwrap_or("Deleted persona");
    match v["variantLabel"].as_str() {
        Some(label) if !label.is_empty() => format!("{name} · {label}"),
        _ => name.into(),
    }
}
fn safe_file(root: &Path, base: &Path, id: &str) -> Result<PathBuf> {
    if id.is_empty() || id.contains('/') || id.contains('\\') || id == ".." {
        return Err("Invalid asset identity".into());
    }
    let p = base
        .join(id)
        .canonicalize()
        .map_err(|_| "Asset unavailable")?;
    let r = root.canonicalize().map_err(|_| "Library unavailable")?;
    if !p.starts_with(r) {
        return Err("Asset outside library".into());
    }
    Ok(p)
}
pub fn portrait(
    store: &Store,
    home: &Path,
    library: &str,
    id: &str,
    persona: bool,
) -> Result<String> {
    let root = root(store, home, library)?;
    let path = if persona {
        let p = safe_file(&root, &root.join("personas"), &format!("{id}.json"))?;
        let v: Value = serde_json::from_slice(&fs::read(p).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        safe_file(
            &root,
            &root.join("personas/avatars"),
            v["avatar"].as_str().ok_or("No portrait")?,
        )?
    } else {
        if id.contains('/') || id.contains('\\') {
            return Err("Invalid character identity".into());
        }
        let p = cards(&root.join("characters"))
            .into_iter()
            .find(|p| p.file_name().is_some_and(|n| n == id))
            .ok_or("No portrait")?;
        safe_file(&root, p.parent().ok_or("No portrait")?, id)?
    };
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("Portrait too large".into());
    }
    // Raster images only. PNG metadata is stripped so card descriptions never cross IPC.
    let image = image::load_from_memory(&bytes)
        .map_err(|_| "Invalid portrait")?
        .thumbnail(320, 400);
    let mut output = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut output, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(format!(
        "data:image/png;base64,{}",
        STANDARD.encode(output.into_inner())
    ))
}

pub fn stats(
    store: &Store,
    home: &Path,
    library: &str,
    character: Option<&str>,
    group: Option<&str>,
) -> Result<Stats> {
    let root = root(store, home, library)?;
    let db = open_readonly(&root.join("chats.db")).map_err(|e| e.to_string())?;
    db.execute_batch("BEGIN").map_err(|e| e.to_string())?;
    let kind = if has(&db, "chats", "kind") {
        "c.kind"
    } else {
        "'direct'"
    };
    let speaker = if has(&db, "messages", "character_id") {
        "COALESCE(m.character_id,c.character_id)"
    } else {
        "c.character_id"
    };
    let persona = if has(&db, "messages", "persona_id") {
        "m.persona_id"
    } else {
        "NULL"
    };
    // Character summary covers direct chats; group participation is returned separately.
    let scope = "(?1 IS NULL OR c.character_id=?1) AND (?2 IS NULL OR c.id=?2)";
    let args: [&dyn rusqlite::ToSql; 2] = [&character, &group];
    let base=format!("WITH msg AS (SELECT m.*,c.created,c.modified,c.metadata,c.character_id AS owner,{kind} AS kind,{speaker} AS speaker,{persona} AS persona,json_extract(m.swipes,'$['||m.swipe_id||']') AS text,json_extract(m.swipe_info,'$['||m.swipe_id||'].send_date') AS sent FROM messages m JOIN chats c ON c.id=m.chat_id WHERE {scope}), swipe AS (SELECT msg.*,s.value AS info,COALESCE(json_extract(s.value,'$.extra.token_count'),0) AS tokens,COALESCE(json_extract(s.value,'$.extra.usage_reported'),0) AS measured,(julianday(json_extract(s.value,'$.gen_finished'))-julianday(json_extract(s.value,'$.gen_started')))*86400000.0 AS elapsed FROM msg,json_each(msg.swipe_info) s WHERE is_user=0 AND is_system=0 AND position>0)");
    let mut totals:Totals=rows(&db,&format!("{base} SELECT (SELECT COUNT(*) FROM chats c WHERE {scope}) AS chats,COUNT(*) AS messages,COALESCE(SUM(is_user),0) AS user_messages,COALESCE(SUM(is_user=0 AND is_system=0),0) AS replies,(SELECT COUNT(*) FROM swipe) AS swipes,(SELECT COALESCE(SUM(CASE WHEN measured=1 THEN tokens ELSE 0 END),0) FROM swipe) AS measured_tokens,(SELECT COALESCE(SUM(CASE WHEN measured=0 THEN tokens ELSE 0 END),0) FROM swipe) AS estimated_tokens,0 AS active_minutes FROM msg"),&args)?.remove(0);
    let active:Vec<Value>=rows(&db,&format!("{base}, stamps AS (SELECT chat_id,position,CAST(strftime('%s',sent) AS INTEGER)*1000 AS ts FROM msg WHERE strftime('%s',sent) IS NOT NULL), gaps AS (SELECT ts-LAG(ts) OVER(PARTITION BY chat_id ORDER BY position) AS gap FROM stamps) SELECT COALESCE(SUM(CASE WHEN gap>0 AND gap<=1800000 THEN gap ELSE 0 END),0) AS ms FROM gaps"),&args)?;
    totals.active_minutes =
        ((active[0]["ms"].as_i64().unwrap_or(0) as f64) / 60000.).round() as i64;
    let mut habits:Habits=rows(&db,&format!("{base} SELECT COALESCE(SUM(CASE WHEN is_user=0 AND is_system=0 AND position>0 THEN MAX(json_array_length(swipes)-1,0) ELSE 0 END),0) AS rerolls,COALESCE(SUM(is_user=0 AND is_system=0 AND position>0 AND json_array_length(swipes)>1),0) AS rerolled_replies,COALESCE(SUM(is_user=0 AND is_system=0 AND position>0),0) AS eligible_replies,COALESCE(AVG(CASE WHEN is_user=1 THEN length(text) END),0) AS avg_user_chars,COALESCE(AVG(CASE WHEN is_user=0 AND is_system=0 THEN length(text) END),0) AS avg_reply_chars,COALESCE(MAX(CASE WHEN is_user=0 AND is_system=0 THEN length(text) ELSE 0 END),0) AS longest_reply_chars,(SELECT COUNT(*) FROM swipe WHERE COALESCE(json_extract(info,'$.extra.reasoning'),'')<>'') AS reasoning_swipes,(SELECT COUNT(*) FROM swipe WHERE elapsed>0) AS timed_swipes,(SELECT COUNT(*) FROM chats c WHERE {scope} AND json_extract(metadata,'$.branchedFrom') IS NOT NULL) AS branched_chats,0 AS deleted_chats FROM msg"),&args)?.remove(0);
    if group.is_none() {
        if let Ok(files) = fs::read_dir(root.join("backups")) {
            for f in files.flatten() {
                if f.path().extension().is_some_and(|s| s == "json") {
                    let v: Value = fs::read(f.path())
                        .ok()
                        .and_then(|b| serde_json::from_slice(&b).ok())
                        .unwrap_or(Value::Null);
                    if v["id"].is_string()
                        && v["messages"].is_array()
                        && character.is_none_or(|id| v["characterId"] == id)
                    {
                        habits.deleted_chats += 1;
                    }
                }
            }
        }
    }
    // chats.db cannot join TokenTrail's model_alias, so the alias segments are
    // inlined: each swipe resolves to the name its model had when it finished.
    let aliases = store.get_model_aliases().map_err(|e| e.to_string())?;
    let display = alias_case_sql(
        &aliases,
        "COALESCE(json_extract(info,'$.extra.model'),'unknown')",
        "CAST((julianday(COALESCE(json_extract(info,'$.gen_finished'),sent,'now'))-2440587.5)*86400000 AS INTEGER)",
    );
    let models:Vec<Model>=rows(&db,&format!("{base} SELECT {display} AS model,COALESCE(json_extract(info,'$.extra.api'),'unknown') AS provider,COUNT(*) AS swipes,SUM(CASE WHEN measured=1 THEN tokens ELSE 0 END) AS measured_tokens,SUM(CASE WHEN measured=0 THEN tokens ELSE 0 END) AS estimated_tokens,COALESCE(SUM(elapsed>0),0) AS timed_swipes,COALESCE(SUM(CASE WHEN elapsed>0 THEN elapsed ELSE 0 END),0) AS total_ms,COALESCE(SUM(CASE WHEN elapsed>0 AND measured=1 THEN elapsed ELSE 0 END),0) AS measured_ms,SUM(CASE WHEN elapsed>0 AND measured=1 THEN tokens ELSE 0 END) AS measured_timed_tokens,COALESCE(SUM(CASE WHEN elapsed>0 AND measured=0 THEN elapsed ELSE 0 END),0) AS estimated_ms,SUM(CASE WHEN elapsed>0 AND measured=0 THEN tokens ELSE 0 END) AS estimated_timed_tokens FROM swipe GROUP BY 1,2 ORDER BY swipes DESC"),&args)?;
    let mut merged: BTreeMap<(String, String), Model> = BTreeMap::new();
    for m in models {
        let key = (m.model.clone(), m.provider.clone());
        if let Some(x) = merged.get_mut(&key) {
            x.swipes += m.swipes;
            x.measured_tokens += m.measured_tokens;
            x.estimated_tokens += m.estimated_tokens;
            x.timed_swipes += m.timed_swipes;
            x.total_ms += m.total_ms;
            x.measured_ms += m.measured_ms;
            x.measured_timed_tokens += m.measured_timed_tokens;
            x.estimated_ms += m.estimated_ms;
            x.estimated_timed_tokens += m.estimated_timed_tokens;
        } else {
            merged.insert(key, m);
        }
    }
    let mut models: Vec<_> = merged.into_values().collect();
    models.sort_by_key(|m| std::cmp::Reverse(m.swipes));
    let mut personas:Vec<Persona>=rows(&db,&format!("{base}, people AS (SELECT persona AS persona_id,0 AS chats,COUNT(*) AS messages FROM msg WHERE is_user=1 GROUP BY persona UNION ALL SELECT json_extract(c.metadata,'$.persona') AS persona_id,COUNT(*),0 FROM chats c WHERE {scope} GROUP BY persona_id) SELECT persona_id,'' AS name,SUM(chats) AS chats,SUM(messages) AS messages FROM people GROUP BY persona_id ORDER BY messages DESC"),&args)?;
    for p in &mut personas {
        p.name = persona_name(&root, p.persona_id.as_deref());
    }
    let hours:Vec<Value>=rows(&db,&format!("{base} SELECT CAST(strftime('%s',sent) AS INTEGER)*1000 AS hour,COUNT(*) AS count FROM msg WHERE strftime('%s',sent) IS NOT NULL GROUP BY hour ORDER BY hour"),&args)?;
    let names = names(&root);
    let chat_sql=format!("SELECT c.id,c.title,{kind} AS kind,c.character_id,c.created,c.modified,(SELECT COUNT(*) FROM messages m WHERE m.chat_id=c.id) AS messages,(SELECT COUNT(*) FROM messages m WHERE m.chat_id=c.id AND m.is_user=0 AND m.is_system=0 AND m.position>0) AS replies,(SELECT COALESCE(SUM(CASE WHEN COALESCE(json_extract(s.value,'$.extra.usage_reported'),0)=1 THEN COALESCE(json_extract(s.value,'$.extra.token_count'),0) ELSE 0 END),0) FROM messages m,json_each(m.swipe_info) s WHERE m.chat_id=c.id AND m.is_user=0 AND m.is_system=0 AND m.position>0) AS measured_tokens,(SELECT COALESCE(SUM(CASE WHEN COALESCE(json_extract(s.value,'$.extra.usage_reported'),0)=0 THEN COALESCE(json_extract(s.value,'$.extra.token_count'),0) ELSE 0 END),0) FROM messages m,json_each(m.swipe_info) s WHERE m.chat_id=c.id AND m.is_user=0 AND m.is_system=0 AND m.position>0) AS estimated_tokens FROM chats c WHERE {scope} ORDER BY messages DESC,c.modified DESC");
    let chats: Vec<Chat> = rows(&db, &chat_sql, &args)?;
    let mut groups: Vec<Chat> = Vec::new();
    if let Some(id) = character {
        if has(&db, "messages", "character_id") {
            let group_scope="c.kind='group' AND EXISTS(SELECT 1 FROM messages m WHERE m.chat_id=c.id AND m.character_id=?1)";
            let q = chat_sql.replace(scope, group_scope).replace(
                "m.chat_id=c.id AND m.is_user=0",
                "m.chat_id=c.id AND m.character_id=?1 AND m.is_user=0",
            );
            groups = rows(&db, &q, &[&id])?;
        }
    } else {
        groups.extend(chats.iter().filter(|c| c.kind == "group").cloned());
    }
    let cast_base=format!("WITH replies AS (SELECT m.*,c.created,c.modified,{speaker} AS character_id_resolved FROM messages m JOIN chats c ON c.id=m.chat_id WHERE m.is_user=0 AND m.is_system=0 AND m.position>0 AND (?1 IS NULL OR {speaker}=?1) AND (?2 IS NULL OR c.id=?2))");
    let mut cast:Vec<Cast>=rows(&db,&format!("{cast_base} SELECT character_id_resolved AS character_id,'' AS name,COUNT(DISTINCT chat_id) AS chats,COUNT(*) AS replies,MIN(created) AS first_chat,MAX(modified) AS last_chat,SUM((SELECT COALESCE(SUM(CASE WHEN COALESCE(json_extract(s.value,'$.extra.usage_reported'),0)=1 THEN COALESCE(json_extract(s.value,'$.extra.token_count'),0) ELSE 0 END),0) FROM json_each(swipe_info) s)) AS measured_tokens,SUM((SELECT COALESCE(SUM(CASE WHEN COALESCE(json_extract(s.value,'$.extra.usage_reported'),0)=0 THEN COALESCE(json_extract(s.value,'$.extra.token_count'),0) ELSE 0 END),0) FROM json_each(swipe_info) s)) AS estimated_tokens FROM replies WHERE character_id_resolved IS NOT NULL GROUP BY character_id_resolved ORDER BY replies DESC,character_id"),&args)?;
    // A character with only greetings or empty chats still belongs in the library.
    let direct:Vec<Cast>=rows(&db,&format!("SELECT c.character_id,'' AS name,COUNT(*) AS chats,0 AS replies,0 AS measured_tokens,0 AS estimated_tokens,MIN(c.created) AS first_chat,MAX(c.modified) AS last_chat FROM chats c WHERE c.character_id IS NOT NULL AND {scope} GROUP BY c.character_id"),&args)?;
    for c in direct {
        if let Some(existing) = cast.iter_mut().find(|x| x.character_id == c.character_id) {
            let group_count: i64 = if has(&db, "messages", "character_id") {
                db.query_row("SELECT COUNT(DISTINCT c.id) FROM chats c JOIN messages m ON m.chat_id=c.id WHERE c.character_id IS NULL AND m.character_id=?1 AND m.is_user=0 AND m.is_system=0 AND m.position>0 AND (?2 IS NULL OR c.id=?2)",params![c.character_id,group],|r|r.get(0)).unwrap_or(0)
            } else {
                0
            };
            existing.chats = c.chats + group_count;
            existing.first_chat = existing.first_chat.min(c.first_chat);
            existing.last_chat = existing.last_chat.max(c.last_chat);
        } else {
            cast.push(c);
        }
    }
    for c in &mut cast {
        c.name = names
            .get(&c.character_id)
            .cloned()
            .unwrap_or_else(|| c.character_id.clone());
    }
    cast.sort_by(|a, b| {
        b.replies
            .cmp(&a.replies)
            .then(a.character_id.cmp(&b.character_id))
    });

    let first_chat = chats.iter().chain(groups.iter()).map(|c| c.created).min();
    let last_chat = chats.iter().chain(groups.iter()).map(|c| c.modified).max();
    let name = character
        .map(|id| names.get(id).cloned().unwrap_or_else(|| id.into()))
        .or_else(|| group.and_then(|id| chats.iter().find(|c| c.id == id).map(|c| c.title.clone())))
        .unwrap_or_else(|| "Your library".into());
    Ok(Stats {
        totals,
        habits,
        cast,
        chats,
        groups,
        models,
        personas,
        hours: hours
            .into_iter()
            .map(|h| {
                (
                    h["hour"].as_i64().unwrap_or(0),
                    h["count"].as_i64().unwrap_or(0),
                )
            })
            .collect(),
        co_creator_sessions: if character.is_none() && group.is_none() {
            scalar(&db, "SELECT COUNT(*) FROM cocreator_sessions")
        } else {
            0
        },
        first_chat,
        last_chat,
        name,
    })
}

#[derive(Serialize, Deserialize, Default)]
pub struct UsageTotal {
    pub events: i64,
    pub measured_events: i64,
    pub priced_events: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub estimated_tokens: i64,
    pub cost_usd: Option<f64>,
    pub avg_duration_ms: Option<f64>,
    pub avg_ttft_ms: Option<f64>,
}
#[derive(Serialize, Deserialize)]
pub struct Breakdown {
    pub name: String,
    pub events: i64,
    pub measured_tokens: i64,
    pub estimated_tokens: i64,
    pub cost_usd: Option<f64>,
}
#[derive(Serialize, Deserialize)]
pub struct Call {
    pub id: String,
    pub ts: i64,
    pub model: String,
    pub purpose: String,
    pub outcome: String,
    pub measured_tokens: i64,
    pub estimated_tokens: i64,
    pub cost_usd: Option<f64>,
    pub duration_ms: Option<i64>,
    pub ttft_ms: Option<i64>,
}
#[derive(Serialize)]
pub struct Usage {
    pub totals: UsageTotal,
    pub features: Vec<Breakdown>,
    pub models: Vec<Breakdown>,
    pub providers: Vec<Breakdown>,
    pub characters: Vec<Breakdown>,
    pub conversations: Vec<Breakdown>,
    pub outcomes: Vec<Breakdown>,
    pub daily: Vec<Breakdown>,
    pub calls: Vec<Call>,
}
pub fn usage(
    store: &Store,
    library: Option<&str>,
    days: i64,
    session: Option<&str>,
) -> Result<Usage> {
    if ![0, 7, 30, 90].contains(&days) {
        return Err("Invalid usage range".into());
    }
    let since = if days == 0 {
        i64::MIN
    } else {
        time::OffsetDateTime::now_utc().unix_timestamp() * 1000 - days * 86_400_000
    };
    let db = store.read_conn();
    let args: [&dyn rusqlite::ToSql; 3] = [&library, &since, &session];
    let base="WITH filtered AS (SELECT u.*,COALESCE(a.canonical,u.model,'unknown') AS display_model FROM usage_event u LEFT JOIN model_alias a ON a.alias=u.model AND u.ts>=a.from_ts AND u.ts<a.until_ts WHERE u.source='wackchatter' AND u.ts>=?2 AND (?1 IS NULL OR EXISTS(SELECT 1 FROM wackchatter_membership m WHERE m.event_id=u.source_event_id AND m.library_id=?1)) AND (?3 IS NULL OR EXISTS(SELECT 1 FROM wackchatter_membership m WHERE m.event_id=u.source_event_id AND m.session_id=?3 AND (?1 IS NULL OR m.library_id=?1)) OR (?1 IS NULL AND u.session_id=?3) OR (?3='unknown' AND u.session_id IS NULL)) AND COALESCE(a.canonical,u.model,'unknown') NOT IN (SELECT name FROM hidden_model))";
    let measured="CASE WHEN estimated=0 THEN input_tokens+output_tokens+cache_read_tokens+cache_write_tokens ELSE 0 END";
    let estimated = "CASE WHEN estimated=1 THEN output_tokens ELSE 0 END";
    let totals:UsageTotal=rows(&db,&format!("{base} SELECT COUNT(*) AS events,COALESCE(SUM(estimated=0),0) AS measured_events,COALESCE(SUM(estimated=0 AND cost_usd IS NOT NULL),0) AS priced_events,COALESCE(SUM(CASE WHEN estimated=0 THEN input_tokens ELSE 0 END),0) AS input_tokens,COALESCE(SUM(CASE WHEN estimated=0 THEN output_tokens ELSE 0 END),0) AS output_tokens,COALESCE(SUM(CASE WHEN estimated=0 THEN cache_read_tokens ELSE 0 END),0) AS cache_read_tokens,COALESCE(SUM(CASE WHEN estimated=0 THEN cache_write_tokens ELSE 0 END),0) AS cache_write_tokens,COALESCE(SUM({estimated}),0) AS estimated_tokens,SUM(CASE WHEN estimated=0 THEN cost_usd END) AS cost_usd,AVG(duration_ms) AS avg_duration_ms,AVG(ttft_ms) AS avg_ttft_ms FROM filtered"),&args)?.remove(0);
    let breakdown = |expr: &str| -> Result<Vec<Breakdown>> {
        rows(&db,&format!("{base} SELECT COALESCE({expr},'unknown') AS name,COUNT(*) AS events,SUM({measured}) AS measured_tokens,SUM({estimated}) AS estimated_tokens,SUM(CASE WHEN estimated=0 THEN cost_usd END) AS cost_usd FROM filtered GROUP BY name ORDER BY events DESC,name"),&args)
    };
    // One generation may have several transcript memberships. Choose a stable display
    // membership without multiplying usage; qualify character identities across libraries.
    let characters=breakdown("(SELECT CASE WHEN ?1 IS NULL THEN m.library_id || ' · ' || m.character_id ELSE m.character_id END FROM wackchatter_membership m WHERE m.event_id=filtered.source_event_id AND (?1 IS NULL OR m.library_id=?1) AND m.character_id<>'' ORDER BY m.rank DESC,m.library_id,m.character_id LIMIT 1)")?;
    let calls = if session.is_some() {
        rows(&db,&format!("{base} SELECT source_event_id AS id,ts,display_model AS model,COALESCE(purpose,'unknown') AS purpose,COALESCE(outcome,'unknown') AS outcome,{measured} AS measured_tokens,{estimated} AS estimated_tokens,CASE WHEN estimated=0 THEN cost_usd END AS cost_usd,duration_ms,ttft_ms FROM filtered ORDER BY ts DESC,source_event_id"),&args)?
    } else {
        Vec::new()
    };
    Ok(Usage {
        totals,
        features: breakdown("purpose")?,
        models: breakdown("display_model")?,
        providers: breakdown("COALESCE(provider_name,provider)")?,
        characters,
        conversations: breakdown("COALESCE((SELECT NULLIF(m.session_id,'') FROM wackchatter_membership m WHERE m.event_id=filtered.source_event_id AND (?1 IS NULL OR m.library_id=?1) AND m.session_id<>'' ORDER BY m.rank DESC,m.library_id,m.session_id LIMIT 1),session_id)")?,
        outcomes: breakdown("outcome")?,
        daily: breakdown("date(ts/1000,'unixepoch')")?,
        calls,
    })
}

fn loopback(raw: &str) -> Result<reqwest::Url> {
    let u = reqwest::Url::parse(raw).map_err(|_| "Invalid WackChatter address")?;
    if u.scheme() != "http"
        || !matches!(u.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
        || !u.username().is_empty()
        || u.password().is_some()
    {
        return Err("WackChatter address must be local".into());
    }
    Ok(u)
}
pub async fn destination(
    home: &Path,
    library: &str,
    chat: Option<&str>,
    character: Option<&str>,
) -> Result<String> {
    let raw = fs::read(home.join(".wackchatter/runtime.json"))
        .map_err(|_| "Start WackChatter with this library to open it.")?;
    let runtime: Value =
        serde_json::from_slice(&raw).map_err(|_| "WackChatter runtime is unavailable")?;
    if runtime["version"] != 1 || runtime["libraryId"] != library {
        return Err("Start WackChatter with the matching library.".into());
    }
    let api = loopback(runtime["apiUrl"].as_str().ok_or("Missing local address")?)?;
    let mut frontend = loopback(
        runtime["frontendUrl"]
            .as_str()
            .ok_or("Missing frontend address")?,
    )?;
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .map_err(|e| e.to_string())?;
    let live: Value = client
        .get(api.join("/api/integration").map_err(|e| e.to_string())?)
        .send()
        .await
        .map_err(|_| "WackChatter is closed. Start it with this library.")?
        .error_for_status()
        .map_err(|_| "Update WackChatter to open chats from TokenTrail.")?
        .json()
        .await
        .map_err(|_| "Invalid WackChatter identity")?;
    if live["libraryId"] != library || live["frontendUrl"] != runtime["frontendUrl"] {
        return Err("WackChatter is running a different library.".into());
    }
    frontend.set_path("/");
    frontend.set_query(None);
    frontend.set_fragment(None);
    frontend.query_pairs_mut().append_pair("ttLibrary", library);
    if let Some(id) = chat {
        frontend.query_pairs_mut().append_pair("ttChat", id);
    } else if let Some(id) = character {
        frontend.query_pairs_mut().append_pair("ttCharacter", id);
    }
    Ok(frontend.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collectors::{test_home, test_store};

    #[test]
    fn alias_case_resolves_by_time_and_quotes_names() {
        use crate::models::ModelAlias;
        let a = |from_ts, until_ts, canonical: &str| ModelAlias {
            alias: "deepseek-flash".into(),
            canonical: canonical.into(),
            from_ts,
            until_ts,
        };
        let aliases = [a(0, 1000, "V4.1"), a(1000, i64::MAX, "Jack's V4.2")];
        let db = rusqlite::Connection::open_in_memory().unwrap();
        let at = |model: &str, ts: i64| -> String {
            let sql = alias_case_sql(&aliases, &format!("'{model}'"), &ts.to_string());
            db.query_row(&format!("SELECT {sql}"), [], |r| r.get(0)).unwrap()
        };
        assert_eq!(at("deepseek-flash", 999), "V4.1");
        assert_eq!(at("deepseek-flash", 1000), "Jack's V4.2");
        assert_eq!(at("other", 1000), "other");
    }
    fn fixture() -> (PathBuf, Store, Connection) {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let tag = format!(
            "wc-stats-{}",
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let home = test_home(&tag);
        let lib = home.join("library");
        fs::create_dir_all(&lib).unwrap();
        fs::create_dir_all(home.join(".wackchatter")).unwrap();
        fs::write(lib.join(".wackchatter"), r#"{"created":"library-one"}"#).unwrap();
        fs::write(
            home.join(".wackchatter/library.json"),
            json!({"dataDir":lib}).to_string(),
        )
        .unwrap();
        let db = Connection::open(lib.join("chats.db")).unwrap();
        db.execute_batch("CREATE TABLE chats(id TEXT PRIMARY KEY,character_id TEXT,kind TEXT,title TEXT,created INTEGER,modified INTEGER,metadata TEXT); CREATE TABLE messages(chat_id TEXT,id TEXT,position INTEGER,is_user INTEGER,is_system INTEGER,swipe_id INTEGER,swipes TEXT,swipe_info TEXT,persona_id TEXT,character_id TEXT); CREATE TABLE cocreator_sessions(id TEXT); INSERT INTO chats VALUES('direct','A.png','direct','A conversation',1000,2000,'{}'),('group',NULL,'group','The ensemble',1000,3000,'{}');").unwrap();
        {
            let store = Store::open(&home.join("usage.db")).unwrap();
            (home, store, db)
        }
    }
    fn message(
        db: &Connection,
        chat: &str,
        id: &str,
        pos: i64,
        user: bool,
        hidden: bool,
        who: Option<&str>,
        persona: Option<&str>,
        swipes: Value,
    ) {
        let text: Vec<&str> = swipes
            .as_array()
            .unwrap()
            .iter()
            .map(|_| if user { "hello" } else { "reply text" })
            .collect();
        db.execute(
            "INSERT INTO messages VALUES(?1,?2,?3,?4,?5,0,?6,?7,?8,?9)",
            params![
                chat,
                id,
                pos,
                user,
                hidden,
                json!(text).to_string(),
                swipes.to_string(),
                persona,
                who
            ],
        )
        .unwrap();
    }
    fn swipe(tokens: i64, reported: bool, time: &str) -> Value {
        json!({"send_date":time,"gen_started":"2026-01-01T12:00:00Z","gen_finished":"2026-01-01T12:00:10Z","extra":{"token_count":tokens,"usage_reported":reported,"model":"test-model","api":"legacy-provider","reasoning":"present"}})
    }
    #[test]
    fn empty_and_missing_character_are_valid() {
        let (h, s, _) = fixture();
        let v = stats(&s, &h, "library-one", None, None).unwrap();
        assert_eq!(v.totals.chats, 2);
        assert_eq!(v.totals.messages, 0);
        assert!(v.models.is_empty());
        assert_eq!(
            stats(&s, &h, "library-one", Some("missing.png"), None)
                .unwrap()
                .totals
                .chats,
            0
        );
    }
    #[test]
    fn stats_parity_and_group_attribution() {
        let (h, s, db) = fixture();
        let a = swipe(100, true, "2026-01-01T12:00:00Z");
        let b = swipe(50, false, "2026-01-01T12:05:00Z");
        message(
            &db,
            "direct",
            "greeting",
            0,
            false,
            false,
            None,
            None,
            json!([a, a, a]),
        );
        message(
            &db,
            "direct",
            "user",
            1,
            true,
            false,
            None,
            Some(""),
            json!([a]),
        );
        message(
            &db,
            "direct",
            "reply",
            2,
            false,
            false,
            None,
            None,
            json!([b, a]),
        );
        message(
            &db,
            "direct",
            "hidden",
            3,
            false,
            true,
            None,
            None,
            json!([a, a]),
        );
        message(&db, "group", "user", 0, true, false, None, None, json!([a]));
        message(
            &db,
            "group",
            "a",
            1,
            false,
            false,
            Some("A.png"),
            None,
            json!([a]),
        );
        message(
            &db,
            "group",
            "b",
            2,
            false,
            false,
            Some("B.png"),
            None,
            json!([b]),
        );
        let v = stats(&s, &h, "library-one", None, None).unwrap();
        assert_eq!(v.totals.messages, 7);
        assert_eq!(v.totals.swipes, 4);
        assert_eq!(v.totals.measured_tokens, 200);
        assert_eq!(v.totals.estimated_tokens, 100);
        assert_eq!(v.habits.rerolls, 1);
        assert_eq!(v.habits.eligible_replies, 3);
        assert_eq!(v.habits.reasoning_swipes, 4);
        assert_eq!(v.habits.timed_swipes, 4);
        assert_eq!(v.totals.active_minutes, 10);
        assert_eq!(v.hours.iter().map(|x| x.1).sum::<i64>(), 7);
        assert_eq!(v.cast[0].character_id, "A.png");
        assert_eq!(v.cast[0].replies, 2);
        assert_eq!(v.cast[0].chats, 2);
        assert_eq!(v.models[0].swipes, 4);
        assert_eq!(v.personas.len(), 2);
        let c = stats(&s, &h, "library-one", Some("A.png"), None).unwrap();
        assert_eq!(c.totals.chats, 1);
        assert_eq!(c.groups.len(), 1);
        assert_eq!(c.groups[0].replies, 1);
        assert_eq!(c.groups[0].measured_tokens, 100);
        let g = stats(&s, &h, "library-one", None, Some("group")).unwrap();
        assert_eq!(g.totals.chats, 1);
        assert_eq!(g.cast.len(), 2);
    }
    #[test]
    fn sessions_drop_long_gaps_and_bad_timing() {
        let (h, s, db) = fixture();
        for (i, t) in [
            "2026-01-01T12:00:00Z",
            "2026-01-01T12:20:00Z",
            "2026-01-01T16:00:00Z",
        ]
        .iter()
        .enumerate()
        {
            let mut sw = swipe(10, true, t);
            sw["gen_finished"] = json!("2025-01-01T00:00:00Z");
            message(
                &db,
                "direct",
                &i.to_string(),
                i as i64 + 1,
                false,
                false,
                None,
                None,
                json!([sw]),
            );
        }
        let v = stats(&s, &h, "library-one", None, None).unwrap();
        assert_eq!(v.totals.active_minutes, 20);
        assert_eq!(v.models[0].timed_swipes, 0);
        assert_eq!(v.models[0].total_ms, 0.);
    }
    #[test]
    fn usage_empty_and_real_all_time() {
        let s = test_store("wc-stats");
        assert_eq!(usage(&s, None, 0, None).unwrap().totals.events, 0);
        assert!(usage(&s, None, 10, None).is_err());
    }
    #[test]
    fn aliases_preserve_library_inventory() {
        let (h, s, db) = fixture();
        let a = swipe(100, true, "2026-01-01T12:00:00Z");
        message(&db, "direct", "r", 1, false, false, None, None, json!([a]));
        s.merge_models(
            &["test-model".to_string(), "canonical".to_string()],
            "canonical",
        )
        .unwrap();
        let v = stats(&s, &h, "library-one", None, None).unwrap();
        assert_eq!(v.models[0].model, "canonical");
    }
    #[test]
    fn portrait_traversal_is_rejected() {
        let (h, s, _) = fixture();
        assert!(portrait(&s, &h, "library-one", "../secrets.json", false).is_err());
        assert!(portrait(&s, &h, "library-one", "../secrets", true).is_err());
    }
    #[test]
    fn loopback_only() {
        for raw in [
            "https://localhost:8787",
            "http://example.com",
            "http://localhost.attacker.test",
            "http://user:pass@localhost",
            "file:///tmp/a",
        ] {
            assert!(loopback(raw).is_err(), "{raw}");
        }
        assert!(loopback("http://127.0.0.1:9999").is_ok());
    }
    #[test]
    fn missing_library_is_not_an_empty_library() {
        let (h, s, _) = fixture();
        assert!(stats(&s, &h, "missing", None, None).is_err());
    }
    #[test]
    fn parity_with_all_original_wackchatter_stats_fixtures() {
        let fixtures: Vec<Value> =
            serde_json::from_str(include_str!("../fixtures/wackchatter_stats_parity.json"))
                .unwrap();
        for (index, sample) in fixtures.iter().enumerate() {
            let (home, store, db) = fixture();
            for table in ["messages", "chats", "cocreator_sessions"] {
                db.execute(&format!("DROP TABLE {table}"), []).unwrap();
                let source = sample["tables"][table].as_array().unwrap();
                let fallback = match table {
                    "chats" => vec![
                        "id",
                        "character_id",
                        "kind",
                        "title",
                        "created",
                        "modified",
                        "metadata",
                    ],
                    "messages" => vec![
                        "chat_id",
                        "id",
                        "position",
                        "is_user",
                        "is_system",
                        "swipe_id",
                        "swipes",
                        "swipe_info",
                        "persona_id",
                        "character_id",
                    ],
                    _ => vec!["id"],
                };
                let columns: Vec<String> = source
                    .first()
                    .and_then(Value::as_object)
                    .map(|o| o.keys().cloned().collect())
                    .unwrap_or_else(|| fallback.into_iter().map(String::from).collect());
                db.execute(&format!("CREATE TABLE {table} ({})", columns.join(",")), [])
                    .unwrap();
                for row in source {
                    let values: Vec<rusqlite::types::Value> = columns
                        .iter()
                        .map(|c| match &row[c] {
                            Value::Null => rusqlite::types::Value::Null,
                            Value::Number(n) => {
                                rusqlite::types::Value::Integer(n.as_i64().unwrap())
                            }
                            Value::String(v) => rusqlite::types::Value::Text(v.clone()),
                            _ => panic!("Unexpected fixture field"),
                        })
                        .collect();
                    db.execute(
                        &format!(
                            "INSERT INTO {table} VALUES({})",
                            vec!["?"; columns.len()].join(",")
                        ),
                        rusqlite::params_from_iter(values),
                    )
                    .unwrap();
                }
            }
            let actual = stats(&store, &home, "library-one", None, None).unwrap();
            let expected = &sample["expected"];
            let t = &expected["totals"];
            for (name, value) in [
                ("chats", actual.totals.chats),
                ("messages", actual.totals.messages),
                ("userMessages", actual.totals.user_messages),
                ("replies", actual.totals.replies),
                ("swipes", actual.totals.swipes),
                (
                    "tokens",
                    actual.totals.measured_tokens + actual.totals.estimated_tokens,
                ),
                ("activeMinutes", actual.totals.active_minutes),
            ] {
                assert_eq!(value, t[name].as_i64().unwrap(), "fixture {index}: {name}");
            }
            // Finer instants preserve local midnight in half/quarter-hour time zones.
            // Folding them to UTC hours still reproduces the original Stats fixture.
            let mut hours = BTreeMap::<i64, i64>::new();
            for (instant, count) in actual.hours {
                *hours
                    .entry(instant.div_euclid(3_600_000) * 3_600_000)
                    .or_default() += count;
            }
            assert_eq!(
                json!(hours.into_iter().collect::<Vec<_>>()),
                expected["hours"],
                "fixture {index}: hours"
            );
            let h = &expected["habits"];
            for (name, value) in [
                ("rerolls", actual.habits.rerolls),
                ("rerolledReplies", actual.habits.rerolled_replies),
                ("eligibleReplies", actual.habits.eligible_replies),
                ("reasoningSwipes", actual.habits.reasoning_swipes),
                ("timedSwipes", actual.habits.timed_swipes),
                ("branchedChats", actual.habits.branched_chats),
                ("longestReplyChars", actual.habits.longest_reply_chars),
            ] {
                assert_eq!(value, h[name].as_i64().unwrap(), "fixture {index}: {name}");
            }
            assert_eq!(
                actual.habits.avg_user_chars.round() as i64,
                h["avgUserChars"].as_i64().unwrap()
            );
            assert_eq!(
                actual.habits.avg_reply_chars.round() as i64,
                h["avgReplyChars"].as_i64().unwrap()
            );
            for m in expected["models"].as_array().unwrap() {
                let model = actual
                    .models
                    .iter()
                    .find(|x| x.model == m["model"] && x.provider == m["api"])
                    .unwrap();
                assert_eq!(model.swipes, m["swipes"].as_i64().unwrap());
                assert_eq!(
                    model.measured_tokens + model.estimated_tokens,
                    m["tokens"].as_i64().unwrap()
                );
            }
        }
    }
    #[test]
    fn portraits_are_recursive_versioned_and_metadata_free() {
        let (home, store, _) = fixture();
        let cards = home.join("library/characters/folder");
        fs::create_dir_all(&cards).unwrap();
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2, 2)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let mut bytes = png.into_inner();
        for (key, name) in [("ccv3", "Version Three"), ("chara", "Version Two")] {
            let mut payload = format!("{key}\0").into_bytes();
            payload.extend(
                STANDARD
                    .encode(
                        json!({"data":{"name":name,"description":"PRIVATE CARD BODY"}}).to_string(),
                    )
                    .bytes(),
            );
            let mut chunk = b"tEXt".to_vec();
            chunk.extend(&payload);
            let mut crc = !0u32;
            for b in &chunk {
                crc ^= u32::from(*b);
                for _ in 0..8 {
                    crc = if crc & 1 == 1 {
                        (crc >> 1) ^ 0xedb88320
                    } else {
                        crc >> 1
                    };
                }
            }
            let mut encoded = (payload.len() as u32).to_be_bytes().to_vec();
            encoded.extend(&chunk);
            encoded.extend((!crc).to_be_bytes());
            bytes.splice(bytes.len() - 12..bytes.len() - 12, encoded);
        }
        fs::write(cards.join("A.png"), &bytes).unwrap();
        assert_eq!(names(&home.join("library"))["A.png"], "Version Three");
        let portrait = portrait(&store, &home, "library-one", "A.png", false).unwrap();
        let decoded = STANDARD
            .decode(portrait.split_once(',').unwrap().1)
            .unwrap();
        assert!(!decoded.windows(4).any(|w| w == b"tEXt"));
        assert!(image::load_from_memory(&decoded).is_ok());
    }

    #[test]
    fn disconnected_and_relocated_libraries_keep_identity() {
        let (home, store, db) = fixture();
        let original = home.join("library");
        store.remember_path("wackchatter:library", &original.to_string_lossy());
        store
            .remember_wackchatter_library(&original.to_string_lossy(), "library-one")
            .unwrap();
        drop(db);
        let moved = home.join("relocated");
        fs::rename(&original, &moved).unwrap();
        let unavailable = libraries(&store, &home);
        assert_eq!(unavailable.len(), 1);
        assert_eq!(unavailable[0].id, "library-one");
        assert!(!unavailable[0].available);
        assert!(stats(&store, &home, "library-one", None, None).is_err());
        fs::write(
            home.join(".wackchatter/library.json"),
            json!({"dataDir":moved}).to_string(),
        )
        .unwrap();
        let available = libraries(&store, &home);
        assert_eq!(available.len(), 1);
        assert!(available[0].available && available[0].current);
        assert_eq!(
            stats(&store, &home, "library-one", None, None)
                .unwrap()
                .totals
                .chats,
            2
        );
    }

    #[test]
    fn runtime_identity_is_verified_before_navigation() {
        use std::io::{Read, Write};
        for matching in [true, false] {
            let (home, _, _) = fixture();
            let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}", server.local_addr().unwrap());
            let runtime =
                json!({"version":1,"libraryId":"library-one","apiUrl":url,"frontendUrl":url});
            fs::write(home.join(".wackchatter/runtime.json"), runtime.to_string()).unwrap();
            let body = json!({"version":1,"libraryId":if matching {"library-one"} else {"another-library"},"frontendUrl":url}).to_string();
            let thread = std::thread::spawn(move || {
                let (mut socket, _) = server.accept().unwrap();
                let mut request = [0u8; 4096];
                let n = socket.read(&mut request).unwrap();
                assert!(String::from_utf8_lossy(&request[..n]).starts_with("GET /api/integration "));
                write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
            });
            let result = tauri::async_runtime::block_on(destination(
                &home,
                "library-one",
                Some("chat with spaces"),
                None,
            ));
            thread.join().unwrap();
            if matching {
                let parsed = reqwest::Url::parse(&result.unwrap()).unwrap();
                assert!(parsed
                    .query_pairs()
                    .any(|(k, v)| k == "ttChat" && v == "chat with spaces"));
            } else {
                assert!(result.unwrap_err().contains("different library"));
            }
            fs::remove_file(home.join(".wackchatter/runtime.json")).unwrap();
            assert!(
                tauri::async_runtime::block_on(destination(&home, "library-one", None, None))
                    .is_err()
            );
        }
    }
}
