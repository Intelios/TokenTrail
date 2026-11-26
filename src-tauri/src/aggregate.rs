use serde::Serialize;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use time::format_description::well_known::Rfc3339;
use time::macros::format_description;

use crate::families;
use crate::store::Store;

/// "Total tokens" = input + output + both cache directions. Reasoning tokens
/// are a subset of output on both Anthropic and OpenAI and never added.
const TOKENS: &str = "(input_tokens + output_tokens + cache_read_tokens + cache_write_tokens)";

/// The key the WackCode page groups calls by. A main-thread call in code mode
/// ({purpose: 'chat'} with no mode — every record written before Chat mode shipped —
/// or mode 'code') is the coding agent and surfaces as 'code'; only a chat-mode
/// main-thread call keeps the 'chat' key, which renders as Chat mode. Every
/// other purpose is mode-independent. Column references are qualified for the
/// `u` alias used by the WackCode queries.
const WC_KEY: &str = "CASE WHEN COALESCE(u.purpose,'chat')='chat' AND COALESCE(u.mode,'code')='code' \
                      THEN 'code' ELSE COALESCE(u.purpose,'chat') END";

/// Housekeeping purposes support the chat rather than do its work; a session's
/// headline model list is built from everything else (main thread, sub-agents).
const WC_WORK: &str = "COALESCE(u.purpose,'chat') NOT IN ('title','compaction','branch_summary','goal_verification','commit_message')";

#[derive(Debug, Serialize)]
pub struct UsagePurposeRow {
    pub purpose: String,
    pub requests: i64,
    pub tokens: i64,
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct WackCodeOutcomeRow {
    pub outcome: String,
    pub events: i64,
}

/// Generic name/events/tokens/cost row for the model, provider and project splits.
#[derive(Debug, Serialize)]
pub struct WackCodeBreakdownRow {
    pub name: String,
    pub events: i64,
    pub tokens: i64,
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct DailyPurposeRow {
    pub date: String,
    pub purpose: String,
    pub tokens: i64,
}

#[derive(Debug, Serialize)]
pub struct WackCodeSessionRow {
    pub session_id: String,
    pub project: String,
    pub models: Vec<String>,
    pub events: i64,
    pub subagents: i64,
    pub tokens: i64,
    pub cost_usd: Option<f64>,
    pub completed: i64,
    pub failed: i64,
    pub cancelled: i64,
    pub first_ts: i64,
    pub last_ts: i64,
    /// 'code' or 'chat'; a WackCode chat belongs to one mode for life. `models`
    /// covers work purposes only (housekeeping stays in the expanded timeline).
    pub mode: String,
}

/// One WackCode model call inside a chat, in timeline order. `purpose` and
/// `subagent_id` are WackCode-only columns (every other source leaves them NULL).
#[derive(Debug, Serialize)]
pub struct WackCodeCallRow {
    pub ts: i64,
    pub purpose: String,
    pub subagent_id: Option<String>,
    pub model: String,
    pub outcome: String,
    pub tokens: i64,
    pub duration_ms: i64,
}

#[derive(Debug, Serialize)]
pub struct WackCodeDetail {
    pub total_tokens: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub events: i64,
    pub sessions: i64,
    pub active_days: i64,
    pub cost_usd: Option<f64>,
    pub first_ts: Option<i64>,
    pub last_ts: Option<i64>,
    pub subagent_events: i64,
    pub subagent_tokens: i64,
    pub by_purpose: Vec<UsagePurposeRow>,
    pub by_outcome: Vec<WackCodeOutcomeRow>,
    /// Tokens/calls split by app mode ('code' covers rows with no mode stamp).
    pub by_mode: Vec<WackCodeBreakdownRow>,
    pub by_model: Vec<WackCodeBreakdownRow>,
    pub by_provider: Vec<WackCodeBreakdownRow>,
    pub by_project: Vec<WackCodeBreakdownRow>,
    pub daily: Vec<DailyPurposeRow>,
    pub sessions_list: Vec<WackCodeSessionRow>,
}

pub fn wackcode_detail(store: &Store, days: i64) -> DbResult<WackCodeDetail> {
    let conn = store.read_conn();
    let cut = cutoff(days);
    let totals_sql = format!(
        "SELECT COALESCE(SUM({T}),0), COALESCE(SUM(input_tokens),0),
                COALESCE(SUM(output_tokens),0), COALESCE(SUM(cache_read_tokens),0),
                COALESCE(SUM(cache_write_tokens),0), COUNT(*),
                COUNT(DISTINCT session_id), SUM(cost_usd), MIN(ts), MAX(ts),
                COALESCE(SUM(CASE WHEN u.is_subagent=1 THEN 1 ELSE 0 END),0),
                COALESCE(SUM(CASE WHEN u.is_subagent=1 THEN {T} ELSE 0 END),0),
                COUNT(DISTINCT date(ts/1000,'unixepoch'))
         FROM usage_event u WHERE u.ts >= ?1 AND u.source='wackcode' AND {H}",
        T = TOKENS,
        H = NOT_HIDDEN
    );
    let t = conn.query_row(&totals_sql, [cut], |r| {
        Ok((
            r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?,
            r.get::<_, i64>(4)?, r.get::<_, i64>(5)?, r.get::<_, i64>(6)?, r.get::<_, Option<f64>>(7)?,
            r.get::<_, Option<i64>>(8)?, r.get::<_, Option<i64>>(9)?, r.get::<_, i64>(10)?,
            r.get::<_, i64>(11)?, r.get::<_, i64>(12)?,
        ))
    })?;

    // by_purpose runs on the connection already held here — asking the store
    // for a second one deadlocks a :memory: database (see read_conn). Rows are
    // grouped by the derived key so code-mode and chat-mode main calls split.
    let by_purpose = {
        let mut stmt = conn.prepare(&format!(
            "SELECT {K}, COUNT(*), SUM({T}), SUM(cost_usd)
             FROM usage_event u WHERE u.ts >= ?1 AND u.source='wackcode' AND {H}
             GROUP BY 1 ORDER BY SUM({T}) DESC",
            K = WC_KEY,
            T = TOKENS,
            H = NOT_HIDDEN
        ))?;
        let rows = stmt
            .query_map([cut], |r| {
                Ok(UsagePurposeRow {
                    purpose: r.get(0)?,
                    requests: r.get(1)?,
                    tokens: r.get(2)?,
                    cost_usd: r.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };

    let by_outcome = {
        let mut stmt = conn.prepare(
            "SELECT COALESCE(outcome,'completed'), COUNT(*)
             FROM usage_event u
             WHERE u.ts >= ?1 AND u.source='wackcode'
             GROUP BY 1 ORDER BY 2 DESC",
        )?;
        let rows = stmt
            .query_map([cut], |r| {
                Ok(WackCodeOutcomeRow { outcome: r.get(0)?, events: r.get(1)? })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };

    // one shared shape for the model / provider / project splits
    let breakdown = |sql: String| -> DbResult<Vec<WackCodeBreakdownRow>> {
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map([cut], |r| {
                Ok(WackCodeBreakdownRow {
                    name: r.get(0)?,
                    events: r.get(1)?,
                    tokens: r.get(2)?,
                    cost_usd: r.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    };
    let by_model = breakdown(format!(
        "SELECT COALESCE(a.canonical, u.model, 'unknown'), COUNT(*), COALESCE(SUM({T}),0), SUM(u.cost_usd)
         FROM usage_event u LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
         WHERE u.ts >= ?1 AND u.source='wackcode' AND {H}
         GROUP BY 1 ORDER BY 3 DESC",
        T = TOKENS,
        H = NOT_HIDDEN
    ))?;
    // Custom WackCode connections have uuid ids; the ledger also writes the
    // display name the user typed, so prefer it and fall back to the raw id.
    let by_provider = breakdown(format!(
        "SELECT COALESCE(u.provider_name, u.provider, 'unknown'), COUNT(*), COALESCE(SUM({T}),0), SUM(u.cost_usd)
         FROM usage_event u
         WHERE u.ts >= ?1 AND u.source='wackcode' AND {H}
         GROUP BY 1 ORDER BY 3 DESC",
        T = TOKENS,
        H = NOT_HIDDEN
    ))?;
    // the mode split includes housekeeping: the panel divides the whole window
    let by_mode = breakdown(format!(
        "SELECT COALESCE(u.mode, 'code'), COUNT(*), COALESCE(SUM({T}),0), SUM(u.cost_usd)
         FROM usage_event u
         WHERE u.ts >= ?1 AND u.source='wackcode' AND {H}
         GROUP BY 1 ORDER BY 3 DESC",
        T = TOKENS,
        H = NOT_HIDDEN
    ))?;
    let by_project = breakdown(format!(
        "SELECT {P}, COUNT(*), COALESCE(SUM({T}),0), SUM(u.cost_usd)
         FROM usage_event u LEFT JOIN project_alias pj ON pj.alias = u.project
         WHERE u.ts >= ?1 AND u.source='wackcode' AND {H}
         GROUP BY 1 ORDER BY 3 DESC",
        P = PROJECT,
        T = TOKENS,
        H = NOT_HIDDEN
    ))?;

    let daily = {
        let mut stmt = conn.prepare(&format!(
            "SELECT date(ts/1000,'unixepoch') AS d, {K}, COALESCE(SUM({T}),0)
             FROM usage_event u WHERE u.ts >= ?1 AND u.source='wackcode' AND {H}
             GROUP BY d, 2 ORDER BY d",
            K = WC_KEY,
            T = TOKENS,
            H = NOT_HIDDEN
        ))?;
        let rows = stmt
            .query_map([cut], |r| {
                Ok(DailyPurposeRow { date: r.get(0)?, purpose: r.get(1)?, tokens: r.get(2)? })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };

    // One row per WackCode chat. SQLite's bare-column rule makes the project
    // come from the row that satisfied MAX(u.ts) — the chat's most recent call
    // — so a project switched mid-chat shows the current one. The model chips
    // name the models that did the work: housekeeping calls (the auto-titler
    // above all) stay visible in the expanded timeline only. A chat belongs to
    // one mode for life, so any chat-mode row settles it.
    let sessions_list = {
        let mut stmt = conn.prepare(&format!(
            "SELECT COALESCE(u.session_id, 'unknown'),
                    {P},
                    GROUP_CONCAT(DISTINCT CASE WHEN {W} THEN COALESCE(a.canonical, u.model, 'unknown') END),
                    COUNT(*),
                    COALESCE(SUM(u.is_subagent), 0),
                    COALESCE(SUM({T}), 0),
                    SUM(u.cost_usd),
                    COALESCE(SUM(outcome='completed'), 0),
                    COALESCE(SUM(outcome='failed'), 0),
                    COALESCE(SUM(outcome='cancelled'), 0),
                    MIN(u.ts),
                    MAX(u.ts),
                    CASE WHEN MAX(u.mode='chat') = 1 THEN 'chat' ELSE 'code' END
             FROM usage_event u
             LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
             LEFT JOIN project_alias pj ON pj.alias = u.project
             WHERE u.ts >= ?1 AND u.source='wackcode' AND {H}
             GROUP BY 1
             ORDER BY MAX(u.ts) DESC",
            P = PROJECT,
            W = WC_WORK,
            T = TOKENS,
            H = NOT_HIDDEN
        ))?;
        let rows = stmt
            .query_map([cut], |r| {
                let models_str: Option<String> = r.get(2)?;
                let mut models: Vec<String> = models_str
                    .map(|s| s.split(',').filter(|m| !m.is_empty()).map(String::from).collect())
                    .unwrap_or_default();
                models.sort();
                Ok(WackCodeSessionRow {
                    session_id: r.get(0)?,
                    project: r.get(1)?,
                    models,
                    events: r.get(3)?,
                    subagents: r.get(4)?,
                    tokens: r.get(5)?,
                    cost_usd: r.get(6)?,
                    completed: r.get(7)?,
                    failed: r.get(8)?,
                    cancelled: r.get(9)?,
                    first_ts: r.get(10)?,
                    last_ts: r.get(11)?,
                    mode: r.get(12)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };

    Ok(WackCodeDetail {
        total_tokens: t.0,
        input_tokens: t.1,
        output_tokens: t.2,
        cache_read_tokens: t.3,
        cache_write_tokens: t.4,
        events: t.5,
        sessions: t.6,
        active_days: t.12,
        cost_usd: t.7,
        first_ts: t.8,
        last_ts: t.9,
        subagent_events: t.10,
        subagent_tokens: t.11,
        by_purpose,
        by_outcome,
        by_mode,
        by_model,
        by_provider,
        by_project,
        daily,
        sessions_list,
    })
}

/// The chronological call timeline behind one chat row. Ignores the range —
/// an expanded chat always shows its whole life, not just the window.
pub fn wackcode_session_calls(store: &Store, session_id: &str) -> DbResult<Vec<WackCodeCallRow>> {
    let conn = store.read_conn();
    let mut stmt = conn.prepare(&format!(
        "SELECT u.ts, {K}, u.subagent_id,
                COALESCE(a.canonical, u.model, 'unknown'),
                COALESCE(u.outcome, 'completed'),
                COALESCE({T}, 0), COALESCE(u.duration_ms, 0)
         FROM usage_event u LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
         WHERE u.source='wackcode' AND u.session_id = ?1 AND {H}
         ORDER BY u.ts",
        K = WC_KEY,
        T = TOKENS,
        H = NOT_HIDDEN
    ))?;
    let rows = stmt
        .query_map([session_id], |r| {
            Ok(WackCodeCallRow {
                ts: r.get(0)?,
                purpose: r.get(1)?,
                subagent_id: r.get(2)?,
                model: r.get(3)?,
                outcome: r.get(4)?,
                tokens: r.get(5)?,
                duration_ms: r.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Rows whose model is user-hidden are excluded from every aggregate. A row is
/// hidden when its model name is hidden, or when it aliases to a hidden
/// canonical name (hiding the display name hides all merged variants). The
/// name compared here is the one the UI displays: NULL-model rows render as
/// 'unknown' everywhere, so they hide when the user hides 'unknown' — a
/// fragment over the raw name (COALESCE(u.model,'')) can never match it.
/// The alias check is per event time: hiding the name a floating API model
/// had before a cutover must not hide what it became after. Correlated rather
/// than using the outer `a` join, so it works in queries without one.
///
/// Every aggregate runs this once per event, so it is shaped for cost: the
/// uncorrelated `IN` list (raw names with *any* segment shown as a hidden name)
/// is built once per query, and the correlated per-time check only runs for
/// rows whose model is on it — almost always none. A NULL model skips the list
/// (NULL NOT IN … is not true) and the correlated check finds no segment for
/// it, exactly as before.
const NOT_HIDDEN: &str = "(COALESCE(u.model, 'unknown') NOT IN (SELECT name FROM hidden_model)
    AND (u.model NOT IN (
            SELECT x.alias FROM model_alias x JOIN hidden_model h ON h.name = x.canonical
        )
        OR NOT EXISTS (
            SELECT 1 FROM model_alias x JOIN hidden_model h ON h.name = x.canonical
            WHERE x.alias = u.model AND u.ts >= x.from_ts AND u.ts < x.until_ts
        )))";

/// "This row displays as model `?N`", for queries about one model. Needs the
/// `a` alias join. The exact test is the `COALESCE`; the `IN` beside it only
/// narrows: a row can show as `?N` only if its raw name is `?N` or an alias
/// with a segment shown as `?N`, plus the NULL-model rows that show as
/// 'unknown'. Without it SQLite reads every event to find one model's rows;
/// with it, it walks `idx_usage_model` for just those names.
const MODEL_IS_1: &str = "COALESCE(a.canonical, u.model, 'unknown') = ?1
    AND (u.model IN (SELECT ?1 UNION SELECT alias FROM model_alias WHERE canonical = ?1)
         OR u.model IS NULL)";
/// `MODEL_IS_1` for queries that bind the model as `?2`.
const MODEL_IS_2: &str = "COALESCE(a.canonical, u.model, 'unknown') = ?2
    AND (u.model IN (SELECT ?2 UNION SELECT alias FROM model_alias WHERE canonical = ?2)
         OR u.model IS NULL)";

/// `MODEL_IS_1`'s exact test alone, for queries over `usage_rows_where(..,
/// &model_rows(1))`, which does the narrowing inside.
const SHOWS_AS_1: &str = "COALESCE(a.canonical, u.model, 'unknown') = ?1";
/// `SHOWS_AS_1` for queries that bind the model as `?2`.
const SHOWS_AS_2: &str = "COALESCE(a.canonical, u.model, 'unknown') = ?2";

/// `MODEL_IS_n`'s narrowing as a `usage_rows_where` predicate over the row
/// alias it is given: rows whose raw name is model `?param` or an alias with a
/// segment shown as it, plus NULL models (shown as 'unknown').
fn model_rows(param: u8) -> impl Fn(&str) -> String {
    move |row| {
        format!(
            "({row}.model IN (SELECT ?{param} UNION SELECT alias FROM model_alias WHERE canonical = ?{param})
              OR {row}.model IS NULL)"
        )
    }
}

/// The project a row counts under: its folder resolved through `project_alias`
/// (folded to its project root at ingest, or folded/kept separate by hand), with
/// NULL landing in the shared 'unknown' bucket. Every query using this needs
/// `LEFT JOIN project_alias pj ON pj.alias = u.project` in its FROM clause, and
/// must group by the evaluated expression — grouping on `u.project` instead would
/// split 'unknown' from NULL all over again.
const PROJECT: &str = "COALESCE(pj.canonical, u.project, 'unknown')";

/// Narrows a query on one named project (never 'unknown') to its own folder and
/// the folders aliased to it, so SQLite walks `idx_usage_project` instead of
/// every event. Only a narrowing: the exact `PROJECT = ?n` test stays beside it.
/// NULL projects show as 'unknown', so they can never match a named project.
/// `row` is the alias of the table it filters (`u` or a `usage_rows_where` row).
fn project_rows(row: &str, param: u8) -> String {
    format!("{row}.project IN (SELECT ?{param} UNION SELECT alias FROM project_alias WHERE canonical = ?{param})")
}

/// Columns of a `usage_rows` row: an event's own, plus `n` (events it stands
/// for) and `min_ts`/`max_ts` (its first and last). A bucket's `ts` is its
/// first event's, which sits in the same UTC hour, day and alias segment as
/// every other event in it — all that `date()`, `strftime('%H')`, the alias
/// join and `NOT_HIDDEN` ever ask of `ts`.
///
/// Every event in a bucket is priced or none is, so its `cost_usd` is NULL
/// exactly when `SUM` over its events would be, and `cost_usd IS NOT NULL`
/// still tells priced rows apart. `reasoning_tokens` is a bucket's sum (0 when
/// none reported, which every query already reads through `COALESCE`).
const ROLLUP_COLS: &str = "r.source, r.model, r.project, r.session_id, r.estimated, r.is_subagent,
    r.min_ts AS ts, r.n, r.input_tokens, r.output_tokens, r.cache_read_tokens, r.cache_write_tokens,
    r.reasoning_tokens, CASE WHEN r.priced = 1 THEN r.cost_sum END AS cost_usd, r.min_ts, r.max_ts";
const EVENT_COLS: &str = "e.source, e.model, e.project, e.session_id, e.estimated, e.is_subagent,
    e.ts, 1 AS n, e.input_tokens, e.output_tokens, e.cache_read_tokens, e.cache_write_tokens,
    e.reasoning_tokens, e.cost_usd, e.ts AS min_ts, e.ts AS max_ts";

/// A bucket whose events fall on both sides of one of its model's alias
/// cutovers (a segment starting or ending after its first event and no later
/// than its last), so they do not all show as one display name. Almost always
/// none: the `IN` list is the aliases that have any cutover at all.
const STRADDLES: &str = "r.model IS NOT NULL
    AND r.model IN (SELECT alias FROM model_alias WHERE from_ts > 0 OR until_ts < 9223372036854775807)
    AND EXISTS (
        SELECT 1 FROM model_alias x WHERE x.alias = r.model
          AND ((x.from_ts > r.min_ts AND x.from_ts <= r.max_ts)
            OR (x.until_ts > r.min_ts AND x.until_ts <= r.max_ts)))";

/// What an aggregate over many events reads in place of `usage_event`: the
/// hourly `usage_rollup` (`store.rs`), which is about one row per busy
/// session-hour instead of one per call, with exact results. Use it as
/// `FROM {usage_rows(..)} u` and write the query as over events, except that
/// a row stands for `u.n` of them: `COUNT(*)` is `SUM(u.n)`, `MIN(ts)` is
/// `MIN(u.min_ts)`, `MAX(ts)` is `MAX(u.max_ts)`, and a per-event count of a
/// column is `SUM(col * u.n)`. Sums of the token and cost columns, distinct
/// sessions, sources, days and hours all read as they did.
///
/// `since` is the SQL for the window's start (a bound parameter, `"?1"`) and
/// replaces the query's own `u.ts >= ?1`; `None` is all time. The window's
/// first, partial hour comes from `usage_event` (one `idx_usage_ts` range), as
/// do the events of a bucket that straddles an alias cutover and anything
/// before 1970, which the rollup leaves out.
///
/// Only for queries that need no other column: anything about purpose, mode,
/// outcome, timing, provider or one event at a time stays on `usage_event`.
fn usage_rows(since: Option<&str>) -> String {
    usage_rows_where(since, &|_| "1".to_string())
}

/// `usage_rows` for a query about one model, family or project: `narrow` is a
/// predicate over a row's own columns, given the alias to write it against,
/// and is applied inside every branch so each reads only the rows that can
/// match (`model_rows`, `family_rows`, `project_rows`). Like `MODEL_IS_n`'s
/// `IN`, it only narrows; the exact display-name test stays in the outer query.
fn usage_rows_where(since: Option<&str>, narrow: &dyn Fn(&str) -> String) -> String {
    let (r, e) = (narrow("r"), narrow("e"));
    let (hours, events) = match since {
        Some(c) => {
            // The first whole hour at or after the window start; before 1970
            // the rollup holds nothing, so from there on it is all events.
            let edge = format!("(CASE WHEN {c} < 0 THEN 0 ELSE ({c} / 3600000 + 1) * 3600000 END)");
            (format!("r.hour >= {edge} / 3600000"), format!("e.ts >= {c} AND e.ts < {edge}"))
        }
        None => ("1".to_string(), "e.ts < 0".to_string()),
    };
    format!(
        "(SELECT {ROLLUP_COLS} FROM usage_rollup r WHERE {hours} AND {r} AND NOT ({STRADDLES})
          UNION ALL
          SELECT {EVENT_COLS} FROM usage_event e WHERE {events} AND {e}
          UNION ALL
          SELECT {EVENT_COLS} FROM usage_rollup r JOIN usage_event e
            ON e.model = r.model AND e.ts >= r.hour * 3600000 AND e.ts < (r.hour + 1) * 3600000
           AND e.source = r.source AND e.project IS r.project AND e.session_id IS r.session_id
           AND e.estimated = r.estimated AND e.is_subagent = r.is_subagent
           AND (e.cost_usd IS NOT NULL) = r.priced
          WHERE {hours} AND {r} AND {STRADDLES})"
    )
}

/// Every visible token since `since`: the denominator of a detail page's share.
fn window_tokens(conn: &rusqlite::Connection, since: i64) -> DbResult<i64> {
    conn.query_row(
        &format!(
            "SELECT COALESCE(SUM({T}),0) FROM {R} u WHERE {H}",
            T = TOKENS,
            H = NOT_HIDDEN,
            R = usage_rows(Some("?1"))
        ),
        [since],
        |r| r.get(0),
    )
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn cutoff(days: i64) -> i64 {
    if days <= 0 {
        0
    } else {
        now_ms() - days * 86_400_000
    }
}

#[derive(Debug, Serialize, Default)]
pub struct SourceTotals {
    pub source: String,
    pub tokens: i64,
    pub events: i64,
    pub sessions: i64,
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct Overview {
    pub total_tokens: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub events: i64,
    pub active_days: i64,
    /// Total tokens divided by the calendar months that have any usage —
    /// the same UTC dates the streaks are computed from, prefixed to YYYY-MM.
    /// Idle gap months are not counted.
    pub avg_monthly_tokens: f64,
    pub months_tracked: i64,
    pub cost_usd: Option<f64>,
    pub first_ts: Option<i64>,
    pub last_ts: Option<i64>,
    pub current_streak: i64,
    pub longest_streak: i64,
    pub by_source: Vec<SourceTotals>,
}

#[derive(Debug, Serialize)]
pub struct DailyRow {
    pub date: String,
    pub source: String,
    pub tokens: i64,
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct DailyModelRow {
    pub date: String,
    pub model: String,
    pub tokens: i64,
}

#[derive(Debug, Serialize)]
pub struct DailyCacheRow {
    pub date: String,
    pub fresh_input: i64,
    pub cache_write: i64,
    pub cache_read: i64,
}

#[derive(Debug, Serialize)]
pub struct ModelRow {
    pub model: String,
    pub tokens: i64,
    pub events: i64,
    pub cost_usd: Option<f64>,
    pub last_ts: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct ProjectRow {
    pub project: String,
    pub tokens: i64,
    pub events: i64,
    pub sessions: i64,
    pub cost_usd: Option<f64>,
    pub first_ts: Option<i64>,
    pub last_ts: Option<i64>,
}

/// A project whose folder is gone, next to a live project that looks like its new
/// home. Never applied automatically: it is a guess and the user decides.
#[derive(Debug, Serialize)]
pub struct ProjectSuggestion {
    pub from: String,
    pub to: String,
    /// `same_name` (a moved folder), `inside` (a dead folder under a live project)
    /// or `subfolder` (a live folder under a live project that never folded).
    pub reason: String,
    /// The suggested folder still exists — `subfolder` always does.
    pub from_exists: bool,
    pub from_last_ts: Option<i64>,
    pub to_last_ts: Option<i64>,
    pub from_tokens: i64,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct DailyProjectRow {
    pub date: String,
    pub project: String,
    pub tokens: i64,
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct HeatmapCell {
    pub date: String,
    pub tokens: i64,
}

#[derive(Debug, Serialize)]
pub struct HourRow {
    pub hour: i64,
    pub tokens: i64,
}

#[derive(Debug, Serialize, PartialEq, Clone)]
pub struct PeakDayRow {
    pub model: String,
    pub date: String,
    pub tokens: i64,
    pub cost_usd: f64,
}

/// One model's contribution to the Wrapped card. No cost field: the card is a
/// token story by design, so the payload never carries a number it must disclaim.
#[derive(Debug, Serialize)]
pub struct WrappedModelRow {
    pub model: String,
    pub tokens: i64,
    pub events: i64,
}

/// Same shape as `SourceTotals` minus cost, for the same reason as above.
#[derive(Debug, Serialize)]
pub struct WrappedSourceRow {
    pub source: String,
    pub tokens: i64,
}

/// Everything the shareable "Usage Wrapped" image draws, for one trailing
/// window (`days` 7/30, or all-time at 0 — the same window semantics as every
/// other ranged query). `window_days` is the span the card compares against:
/// the request itself for week/month, first-event-to-today for all-time.
/// `estimated_tokens` rides along so the card can flag the WackChatter share
/// instead of letting it pass as measured.
#[derive(Debug, Serialize)]
pub struct WrappedSummary {
    pub period_days: i64,
    pub total_tokens: i64,
    pub events: i64,
    pub sessions: i64,
    pub active_days: i64,
    pub window_days: i64,
    pub current_streak: i64,
    pub longest_streak: i64,
    pub first_ts: Option<i64>,
    pub last_ts: Option<i64>,
    pub top_models: Vec<WrappedModelRow>,
    pub by_source: Vec<WrappedSourceRow>,
    pub daily: Vec<HeatmapCell>,
    pub peak_day: Option<String>,
    pub peak_day_tokens: i64,
    pub busiest_hour: Option<i64>,
    /// Share of tokens written between 22:00 and 06:00 UTC, 0.0–1.0.
    pub night_share: f64,
    pub top_project: Option<String>,
    pub top_project_tokens: i64,
    pub estimated_tokens: i64,
}

#[derive(Debug, Serialize)]
pub struct ModelStatsRow {
    pub model: String,
    pub tokens: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub events: i64,
    pub sessions: i64,
    pub cost_usd: Option<f64>,
    pub first_ts: Option<i64>,
    pub last_ts: Option<i64>,
    pub sources: Vec<String>,
    pub active_days: i64,
}

#[derive(Debug, Serialize)]
pub struct ModelDetail {
    pub model: String,
    pub tokens: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub events: i64,
    pub sessions: i64,
    pub cost_usd: Option<f64>,
    pub first_ts: Option<i64>,
    pub last_ts: Option<i64>,
    pub active_days: i64,
    pub current_streak: i64,
    pub longest_streak: i64,
    pub peak_day: Option<String>,
    pub peak_day_tokens: i64,
    pub by_source: Vec<SourceTotals>,
    pub by_project: Vec<ProjectRow>,
    pub daily: Vec<HeatmapCell>,
    pub total_window_tokens: i64,
}

#[derive(Debug, Serialize)]
pub struct TimingSource {
    pub source: String,
    pub timed_calls: i64,
}

#[derive(Debug, Serialize, Default)]
pub struct TimingStats {
    pub timed_calls: i64,
    pub ttft_calls: i64,
    pub generation_calls: i64,
    pub avg_duration_ms: Option<f64>,
    pub avg_ttft_ms: Option<f64>,
    pub end_to_end_output_tps: Option<f64>,
    pub generation_output_tps: Option<f64>,
    pub sources: Vec<TimingSource>,
}

#[derive(Debug, Serialize)]
pub struct ComparisonDailyRow {
    pub date: String,
    pub a_tokens: i64,
    pub b_tokens: i64,
}

#[derive(Debug, Serialize)]
pub struct LeadChange {
    pub date: String,
    pub direction: String,
}

#[derive(Debug, Serialize)]
pub struct DailyGap {
    pub date: String,
    pub wider_side: String,
    pub margin: i64,
}

#[derive(Debug, Serialize, Default)]
pub struct Rivalry {
    pub a_wins: i64,
    pub b_wins: i64,
    pub ties: i64,
    pub lead_changes: Vec<LeadChange>,
    pub latest_date: Option<String>,
    pub latest_direction: Option<String>,
    pub biggest_gap: Option<DailyGap>,
}

#[derive(Debug, Serialize)]
pub struct CostComparison {
    pub anchor_tokens: i64,
    pub a_anchor_cost_usd: f64,
    pub b_anchor_cost_usd: f64,
    pub b_equivalent_tokens: Option<f64>,
}

#[derive(Debug, Serialize, Default)]
pub struct CompareModel {
    pub model: String,
    pub tokens: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub generated_output: i64,
    pub reasoning_tokens: i64,
    pub events: i64,
    pub sessions: i64,
    pub subagent_tokens: i64,
    pub first_ts: Option<i64>,
    pub last_ts: Option<i64>,
    pub active_days: i64,
    pub current_streak: i64,
    pub longest_streak: i64,
    pub peak_day: Option<String>,
    pub peak_day_tokens: i64,
    pub measured_tokens: i64,
    pub estimated_tokens: i64,
    pub priced_tokens: i64,
    pub priced_events: i64,
    pub priced_cost_usd: Option<f64>,
    pub cost_per_mtok: Option<f64>,
    pub hourly: Vec<HourRow>,
    pub busiest_hour: Option<i64>,
    pub night_share: f64,
    pub by_source: Vec<SourceTotals>,
    pub by_project: Vec<ProjectRow>,
    pub timing: TimingStats,
}

#[derive(Debug, Serialize)]
pub struct ModelComparison {
    pub a: CompareModel,
    pub b: CompareModel,
    pub daily: Vec<ComparisonDailyRow>,
    pub rivalry: Rivalry,
    pub total_window_tokens: i64,
    pub cost_comparison: Option<CostComparison>,
}

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct Achievement {
    pub kind: String,
    pub tier: Option<String>,
    pub title: String,
    pub value: String,
    pub earned_ts: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct ProjectModelRow {
    pub model: String,
    pub tokens: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub events: i64,
    pub sessions: i64,
    pub cost_usd: Option<f64>,
    pub first_ts: Option<i64>,
    pub last_ts: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct SessionRow {
    pub session_id: String,
    pub source: String,
    pub events: i64,
    pub tokens: i64,
    pub cost_usd: Option<f64>,
    pub first_ts: i64,
    pub last_ts: i64,
    pub models: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ProjectDetail {
    pub project: String,
    pub tokens: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub events: i64,
    pub sessions: i64,
    pub cost_usd: Option<f64>,
    pub first_ts: Option<i64>,
    pub last_ts: Option<i64>,
    pub active_days: i64,
    pub current_streak: i64,
    pub longest_streak: i64,
    pub peak_day: Option<String>,
    pub peak_day_tokens: i64,
    pub by_model: Vec<ProjectModelRow>,
    pub by_source: Vec<SourceTotals>,
    pub daily: Vec<HeatmapCell>,
    pub sessions_list: Vec<SessionRow>,
    pub total_window_tokens: i64,
}

#[derive(Debug, Serialize)]
pub struct FamilyStatsRow {
    pub family: String,
    pub tokens: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub events: i64,
    pub sessions: i64,
    pub cost_usd: Option<f64>,
    pub first_ts: Option<i64>,
    pub last_ts: Option<i64>,
    pub sources: Vec<String>,
    pub models: Vec<ModelStatsRow>,
}

#[derive(Debug, Serialize)]
pub struct FamilyDetail {
    pub family: String,
    pub tokens: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub events: i64,
    pub sessions: i64,
    pub cost_usd: Option<f64>,
    pub first_ts: Option<i64>,
    pub last_ts: Option<i64>,
    pub active_days: i64,
    pub current_streak: i64,
    pub longest_streak: i64,
    pub peak_day: Option<String>,
    pub peak_day_tokens: i64,
    pub models: Vec<ModelStatsRow>,
    pub by_source: Vec<SourceTotals>,
    pub by_project: Vec<ProjectRow>,
    pub daily: Vec<HeatmapCell>,
    pub total_window_tokens: i64,
}

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct LeaderboardEvent {
    pub kind: String,
    pub model: String,
    pub other_model: Option<String>,
    pub rank: Option<i64>,
    pub date: String,
    pub tokens: i64,
    pub tenure_days: Option<i64>,
}

type DbResult<T> = Result<T, rusqlite::Error>;

pub fn overview(store: &Store) -> DbResult<Overview> {
    // Before `read_conn`: an in-memory store's only connection is the writer,
    // and taking it twice on one thread would deadlock.
    let standings = daily_standings(store)?;
    let conn = store.read_conn();
    struct Totals {
        total: i64,
        input: i64,
        output: i64,
        cr: i64,
        cw: i64,
        events: i64,
        cost: Option<f64>,
        first_ts: Option<i64>,
        last_ts: Option<i64>,
    }
    let mut t = Totals {
        total: 0,
        input: 0,
        output: 0,
        cr: 0,
        cw: 0,
        events: 0,
        cost: None,
        first_ts: None,
        last_ts: None,
    };

    // One pass: the grand totals are the per-source rows added up (SUM skips
    // NULL costs, so the total is NULL only when every source's is).
    let source_sql = format!(
        "SELECT source, COALESCE(SUM({T}),0), SUM(u.n), COUNT(DISTINCT session_id), SUM(cost_usd),
                COALESCE(SUM(input_tokens),0), COALESCE(SUM(output_tokens),0),
                COALESCE(SUM(cache_read_tokens),0), COALESCE(SUM(cache_write_tokens),0),
                MIN(u.min_ts), MAX(u.max_ts)
         FROM {R} u WHERE {H} GROUP BY source ORDER BY 2 DESC",
        T = TOKENS,
        H = NOT_HIDDEN,
        R = usage_rows(None)
    );
    let mut stmt = conn.prepare(&source_sql)?;
    let by_source = stmt
        .query_map([], |r| {
            let row = SourceTotals {
                source: r.get(0)?,
                tokens: r.get(1)?,
                events: r.get(2)?,
                sessions: r.get(3)?,
                cost_usd: r.get(4)?,
            };
            let (first, last): (Option<i64>, Option<i64>) = (r.get(9)?, r.get(10)?);
            t.total += row.tokens;
            t.events += row.events;
            t.input += r.get::<_, i64>(5)?;
            t.output += r.get::<_, i64>(6)?;
            t.cr += r.get::<_, i64>(7)?;
            t.cw += r.get::<_, i64>(8)?;
            if let Some(cost) = row.cost_usd {
                t.cost = Some(t.cost.unwrap_or(0.0) + cost);
            }
            t.first_ts = t.first_ts.into_iter().chain(first).min();
            t.last_ts = t.last_ts.into_iter().chain(last).max();
            Ok(row)
        })?
        .collect::<Result<Vec<_>, _>>()?;

    // Active days are exactly the memoised standings' days.
    let mut dates: Vec<String> = standings.days.keys().cloned().collect();

    let (current, longest) = streaks(&dates);
    let active_days = dates.len() as i64;
    let months_tracked = dates
        .iter()
        .map(|d| &d[..7])
        .collect::<std::collections::BTreeSet<_>>()
        .len() as i64;
    dates.clear();
    let avg_monthly_tokens = if months_tracked > 0 {
        t.total as f64 / months_tracked as f64
    } else {
        0.0
    };

    Ok(Overview {
        total_tokens: t.total,
        input_tokens: t.input,
        output_tokens: t.output,
        cache_read_tokens: t.cr,
        cache_write_tokens: t.cw,
        events: t.events,
        active_days,
        avg_monthly_tokens,
        months_tracked,
        cost_usd: t.cost,
        first_ts: t.first_ts,
        last_ts: t.last_ts,
        current_streak: current,
        longest_streak: longest,
        by_source,
    })
}

/// Count of distinct UTC dates with at least one non-hidden event in the window.
pub fn active_days_for_range(store: &Store, days: i64) -> DbResult<i64> {
    let sql = format!(
        "SELECT COUNT(DISTINCT date(ts/1000, 'unixepoch'))
         FROM {R} u WHERE {H}",
        H = NOT_HIDDEN,
        R = usage_rows(Some("?1"))
    );
    store.read_conn().query_row(&sql, [cutoff(days)], |r| r.get(0))
}

pub fn daily(store: &Store, days: i64) -> DbResult<Vec<DailyRow>> {
    let sql = format!(
        "SELECT date(ts/1000,'unixepoch') AS d, source, COALESCE(SUM({T}),0), SUM(cost_usd)
         FROM {R} u WHERE {H} GROUP BY d, source ORDER BY d",
        T = TOKENS,
        H = NOT_HIDDEN,
        R = usage_rows(Some("?1"))
    );
    let conn = store.read_conn();
    let mut stmt = conn.prepare(&sql)?;
    let rows: Vec<DailyRow> = stmt
        .query_map([cutoff(days)], |r| {
            Ok(DailyRow { date: r.get(0)?, source: r.get(1)?, tokens: r.get(2)?, cost_usd: r.get(3)? })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn daily_by_model(store: &Store, days: i64) -> DbResult<Vec<DailyModelRow>> {
    let sql = format!(
        "SELECT date(ts/1000,'unixepoch') AS d, COALESCE(a.canonical, u.model, 'unknown'),
                COALESCE(SUM({T}),0)
         FROM {R} u LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
         WHERE {H} GROUP BY 1, 2 ORDER BY d",
        T = TOKENS,
        H = NOT_HIDDEN,
        R = usage_rows(Some("?1"))
    );
    let conn = store.read_conn();
    let mut stmt = conn.prepare(&sql)?;
    let rows: Vec<DailyModelRow> = stmt
        .query_map([cutoff(days)], |r| {
            Ok(DailyModelRow { date: r.get(0)?, model: r.get(1)?, tokens: r.get(2)? })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn daily_cache(store: &Store, days: i64) -> DbResult<Vec<DailyCacheRow>> {
    let conn = store.read_conn();
    let mut stmt = conn.prepare(&format!(
        "SELECT date(ts/1000,'unixepoch') AS d,
                COALESCE(SUM(input_tokens),0), COALESCE(SUM(cache_write_tokens),0),
                COALESCE(SUM(cache_read_tokens),0)
         FROM {R} u WHERE {H} GROUP BY d ORDER BY d",
        H = NOT_HIDDEN,
        R = usage_rows(Some("?1"))
    ))?;
    let rows: Vec<DailyCacheRow> = stmt
        .query_map([cutoff(days)], |r| {
            Ok(DailyCacheRow { date: r.get(0)?, fresh_input: r.get(1)?, cache_write: r.get(2)?, cache_read: r.get(3)? })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn by_model(store: &Store, days: i64) -> DbResult<Vec<ModelRow>> {
    let sql = format!(
        "SELECT COALESCE(a.canonical, u.model, 'unknown'), COALESCE(SUM({T}),0), SUM(u.n),
                SUM(cost_usd), MAX(u.max_ts)
         FROM {R} u LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
         WHERE {H} GROUP BY 1 ORDER BY 2 DESC",
        T = TOKENS,
        H = NOT_HIDDEN,
        R = usage_rows(Some("?1"))
    );
    let conn = store.read_conn();
    let mut stmt = conn.prepare(&sql)?;
    let rows: Vec<ModelRow> = stmt
        .query_map([cutoff(days)], |r| {
            Ok(ModelRow { model: r.get(0)?, tokens: r.get(1)?, events: r.get(2)?, cost_usd: r.get(3)?, last_ts: r.get(4)? })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn model_stats(store: &Store, days: i64) -> DbResult<Vec<ModelStatsRow>> {
    let sql = format!(
        "SELECT COALESCE(a.canonical, u.model, 'unknown'),
                COALESCE(SUM(u.input_tokens),0), COALESCE(SUM(u.output_tokens),0),
                COALESCE(SUM(u.cache_read_tokens),0), COALESCE(SUM(u.cache_write_tokens),0),
                COALESCE(SUM({T}),0), SUM(u.n), COUNT(DISTINCT u.session_id),
                SUM(u.cost_usd), MIN(u.min_ts), MAX(u.max_ts), GROUP_CONCAT(DISTINCT u.source),
                COUNT(DISTINCT date(u.ts/1000, 'unixepoch'))
         FROM {R} u LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
         WHERE {H} GROUP BY 1 ORDER BY 6 DESC",
        T = TOKENS,
        H = NOT_HIDDEN,
        R = usage_rows(Some("?1"))
    );
    let conn = store.read_conn();
    let mut stmt = conn.prepare(&sql)?;
    let rows: Vec<ModelStatsRow> = stmt
        .query_map([cutoff(days)], |r| {
            let src_str: Option<String> = r.get(11)?;
            let sources = src_str
                .map(|s| s.split(',').map(String::from).collect())
                .unwrap_or_default();
            Ok(ModelStatsRow {
                model: r.get(0)?,
                input_tokens: r.get(1)?,
                output_tokens: r.get(2)?,
                cache_read_tokens: r.get(3)?,
                cache_write_tokens: r.get(4)?,
                tokens: r.get(5)?,
                events: r.get(6)?,
                sessions: r.get(7)?,
                cost_usd: r.get(8)?,
                first_ts: r.get(9)?,
                last_ts: r.get(10)?,
                sources,
                active_days: r.get(12)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Full detail card for a single model, keyed by display name, filtered by range in days.
/// Returns `None` when the model has no visible usage events.
pub fn model_detail(store: &Store, model: &str, days: i64) -> DbResult<Option<ModelDetail>> {
    let exists_sql = format!(
        "SELECT COALESCE(a.canonical, u.model, 'unknown')
         FROM usage_event u LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
         WHERE {MODEL_IS_1} AND {H}
         LIMIT 1",
        H = NOT_HIDDEN
    );
    let conn = store.read_conn();
    let mut exists_stmt = conn.prepare(&exists_sql)?;
    let canonical_name = match exists_stmt.query_row(rusqlite::params![model], |r| r.get::<_, String>(0)) {
        Ok(name) => name,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
        Err(e) => return Err(e),
    };

    let cutoff_ts = cutoff(days);
    let rows = usage_rows_where(Some("?1"), &model_rows(2));

    let sql = format!(
        "SELECT COALESCE(a.canonical, u.model, 'unknown'),
                COALESCE(SUM(u.input_tokens),0),
                COALESCE(SUM(CASE WHEN u.source = 'antigravity' THEN u.output_tokens + COALESCE(u.reasoning_tokens,0) ELSE u.output_tokens END),0),
                COALESCE(SUM(u.cache_read_tokens),0), COALESCE(SUM(u.cache_write_tokens),0),
                COALESCE(SUM(u.reasoning_tokens),0),
                COALESCE(SUM({T}),0), SUM(u.n), COUNT(DISTINCT u.session_id),
                SUM(u.cost_usd), MIN(u.min_ts), MAX(u.max_ts)
         FROM {rows} u LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
         WHERE {SHOWS_AS_2} AND {H}
         GROUP BY 1",
        T = TOKENS,
        H = NOT_HIDDEN
    );
    let mut stmt = conn.prepare(&sql)?;
    let row = stmt.query_row(rusqlite::params![cutoff_ts, model], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, i64>(4)?,
            r.get::<_, i64>(5)?,
            r.get::<_, i64>(6)?,
            r.get::<_, i64>(7)?,
            r.get::<_, i64>(8)?,
            r.get::<_, Option<f64>>(9)?,
            r.get::<_, Option<i64>>(10)?,
            r.get::<_, Option<i64>>(11)?,
        ))
    });

    let (name, inp, out, cr, cw, reasoning, total, events, sessions, cost, first_ts, last_ts) = match row {
        Ok(r) => r,
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            (canonical_name, 0, 0, 0, 0, 0, 0, 0, 0, None, None, None)
        }
        Err(e) => return Err(e),
    };

    // by source
    let sql_src = format!(
        "SELECT u.source, COALESCE(SUM({T}),0), SUM(u.n), COUNT(DISTINCT u.session_id), SUM(u.cost_usd)
         FROM {rows} u LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
         WHERE {SHOWS_AS_2} AND {H}
         GROUP BY u.source ORDER BY 2 DESC",
        T = TOKENS,
        H = NOT_HIDDEN
    );
    let mut stmt_src = conn.prepare(&sql_src)?;
    let by_source: Vec<SourceTotals> = stmt_src
        .query_map(rusqlite::params![cutoff_ts, model], |r| {
            Ok(SourceTotals {
                source: r.get(0)?,
                tokens: r.get(1)?,
                events: r.get(2)?,
                sessions: r.get(3)?,
                cost_usd: r.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    // by project
    let sql_proj = format!(
        "SELECT {P}, COALESCE(SUM({T}),0), SUM(u.n),
                COUNT(DISTINCT u.session_id), SUM(u.cost_usd), MIN(u.min_ts), MAX(u.max_ts)
         FROM {rows} u
         LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
         LEFT JOIN project_alias pj ON pj.alias = u.project
         WHERE {SHOWS_AS_2} AND {H}
         GROUP BY 1 ORDER BY 2 DESC",
        T = TOKENS,
        H = NOT_HIDDEN,
        P = PROJECT
    );
    let mut stmt_proj = conn.prepare(&sql_proj)?;
    let by_project: Vec<ProjectRow> = stmt_proj
        .query_map(rusqlite::params![cutoff_ts, model], |r| {
            Ok(ProjectRow {
                project: r.get(0)?,
                tokens: r.get(1)?,
                events: r.get(2)?,
                sessions: r.get(3)?,
                cost_usd: r.get(4)?,
                first_ts: r.get(5)?,
                last_ts: r.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    // daily series → active days, streaks, peak day
    let sql_daily = format!(
        "SELECT date(ts/1000,'unixepoch') AS d, COALESCE(SUM({T}),0)
         FROM {rows} u LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
         WHERE {SHOWS_AS_2} AND {H}
         GROUP BY d ORDER BY d",
        T = TOKENS,
        H = NOT_HIDDEN
    );
    let mut stmt_daily = conn.prepare(&sql_daily)?;
    let daily: Vec<HeatmapCell> = stmt_daily
        .query_map(rusqlite::params![cutoff_ts, model], |r| {
            Ok(HeatmapCell { date: r.get(0)?, tokens: r.get(1)? })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let dates: Vec<String> = daily.iter().map(|d| d.date.clone()).collect();
    let active_days = dates.len() as i64;
    let (current_streak, longest_streak) = streaks(&dates);
    let peak = daily.iter().max_by_key(|d| d.tokens);

    let total_window_tokens = window_tokens(&conn, cutoff_ts)?;

    Ok(Some(ModelDetail {
        model: name,
        tokens: total,
        input_tokens: inp,
        output_tokens: out,
        reasoning_tokens: reasoning,
        cache_read_tokens: cr,
        cache_write_tokens: cw,
        events,
        sessions,
        cost_usd: cost,
        first_ts,
        last_ts,
        active_days,
        current_streak,
        longest_streak,
        peak_day: peak.map(|p| p.date.clone()),
        peak_day_tokens: peak.map(|p| p.tokens).unwrap_or(0),
        by_source,
        by_project,
        daily,
        total_window_tokens,
    }))
}

fn finite(value: Option<f64>) -> Option<f64> {
    value.filter(|v| v.is_finite())
}

fn blended_rate(priced_tokens: i64, priced_cost_usd: Option<f64>) -> Option<f64> {
    if priced_tokens <= 0 {
        return None;
    }
    finite(priced_cost_usd.map(|cost| cost / priced_tokens as f64))
}

fn comparison_timing(
    conn: &rusqlite::Connection,
    model: &str,
    cutoff_ts: i64,
) -> DbResult<TimingStats> {
    let eligible = "u.estimated = 0 AND u.duration_ms > 0
                    AND (u.outcome IS NULL OR u.outcome NOT IN ('failed','cancelled'))";
    let generated = "CASE WHEN u.source = 'antigravity'
                          THEN u.output_tokens + COALESCE(u.reasoning_tokens,0)
                          ELSE u.output_tokens END";
    let sql = format!(
        "SELECT COUNT(*),
                COALESCE(SUM(CASE WHEN u.ttft_ms > 0 THEN 1 ELSE 0 END),0),
                COALESCE(SUM(CASE WHEN u.ttft_ms > 0 AND u.duration_ms > u.ttft_ms THEN 1 ELSE 0 END),0),
                AVG(u.duration_ms),
                AVG(CASE WHEN u.ttft_ms > 0 THEN u.ttft_ms END),
                COALESCE(SUM({G}),0), COALESCE(SUM(u.duration_ms),0),
                COALESCE(SUM(CASE WHEN u.ttft_ms > 0 AND u.duration_ms > u.ttft_ms THEN {G} ELSE 0 END),0),
                COALESCE(SUM(CASE WHEN u.ttft_ms > 0 AND u.duration_ms > u.ttft_ms THEN u.duration_ms - u.ttft_ms ELSE 0 END),0)
         FROM usage_event u
         LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
         WHERE u.ts >= ?1 AND {MODEL_IS_2}
           AND {H} AND {E}",
        G = generated,
        H = NOT_HIDDEN,
        E = eligible
    );
    let mut stats = conn.query_row(&sql, rusqlite::params![cutoff_ts, model], |r| {
        let timed_calls = r.get::<_, i64>(0)?;
        let generation_calls = r.get::<_, i64>(2)?;
        let generated_output = r.get::<_, i64>(5)?;
        let duration_ms = r.get::<_, i64>(6)?;
        let generation_output = r.get::<_, i64>(7)?;
        let generation_ms = r.get::<_, i64>(8)?;
        let end_to_end_output_tps = (timed_calls > 0 && duration_ms > 0)
            .then(|| generated_output as f64 * 1000.0 / duration_ms as f64);
        let generation_output_tps = (generation_calls > 0 && generation_ms > 0)
            .then(|| generation_output as f64 * 1000.0 / generation_ms as f64);
        Ok(TimingStats {
            timed_calls,
            ttft_calls: r.get(1)?,
            generation_calls,
            avg_duration_ms: finite(r.get(3)?),
            avg_ttft_ms: finite(r.get(4)?),
            end_to_end_output_tps: finite(end_to_end_output_tps),
            generation_output_tps: finite(generation_output_tps),
            sources: Vec::new(),
        })
    })?;

    let mut stmt = conn.prepare(&format!(
        "SELECT u.source, COUNT(*)
         FROM usage_event u
         LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
         WHERE u.ts >= ?1 AND {MODEL_IS_2}
           AND {H} AND {E}
         GROUP BY u.source ORDER BY 2 DESC, 1",
        H = NOT_HIDDEN,
        E = eligible
    ))?;
    stats.sources = stmt
        .query_map(rusqlite::params![cutoff_ts, model], |r| {
            Ok(TimingSource { source: r.get(0)?, timed_calls: r.get(1)? })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(stats)
}

fn comparison_side(
    conn: &rusqlite::Connection,
    model: &str,
    cutoff_ts: i64,
) -> DbResult<(CompareModel, Vec<HeatmapCell>)> {
    let rows = usage_rows_where(Some("?1"), &model_rows(2));
    let totals_sql = format!(
        "SELECT COALESCE(SUM(u.input_tokens),0), COALESCE(SUM(u.output_tokens),0),
                COALESCE(SUM(u.cache_read_tokens),0), COALESCE(SUM(u.cache_write_tokens),0),
                COALESCE(SUM(CASE WHEN u.source = 'antigravity'
                                  THEN u.output_tokens + COALESCE(u.reasoning_tokens,0)
                                  ELSE u.output_tokens END),0),
                COALESCE(SUM(u.reasoning_tokens),0), COALESCE(SUM({T}),0), COALESCE(SUM(u.n),0),
                COUNT(DISTINCT u.source || char(31) || u.session_id),
                COALESCE(SUM(CASE WHEN u.is_subagent = 1 THEN {T} ELSE 0 END),0),
                MIN(u.min_ts), MAX(u.max_ts),
                COALESCE(SUM(CASE WHEN u.estimated = 0 THEN {T} ELSE 0 END),0),
                COALESCE(SUM(CASE WHEN u.estimated = 1 THEN {T} ELSE 0 END),0),
                COALESCE(SUM(CASE WHEN u.estimated = 0 AND u.cost_usd IS NOT NULL THEN {T} ELSE 0 END),0),
                COALESCE(SUM(CASE WHEN u.estimated = 0 AND u.cost_usd IS NOT NULL THEN u.n ELSE 0 END),0),
                SUM(CASE WHEN u.estimated = 0 AND u.cost_usd IS NOT NULL THEN u.cost_usd END)
         FROM {rows} u
         LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
         WHERE {SHOWS_AS_2} AND {H}",
        T = TOKENS,
        H = NOT_HIDDEN
    );
    let mut side = conn.query_row(&totals_sql, rusqlite::params![cutoff_ts, model], |r| {
        let priced_tokens = r.get::<_, i64>(14)?;
        let priced_cost_usd = finite(r.get::<_, Option<f64>>(16)?);
        let cost_per_mtok = blended_rate(priced_tokens, priced_cost_usd)
            .and_then(|rate| finite(Some(rate * 1e6)));
        Ok(CompareModel {
            model: model.to_string(),
            input_tokens: r.get(0)?,
            output_tokens: r.get(1)?,
            cache_read_tokens: r.get(2)?,
            cache_write_tokens: r.get(3)?,
            generated_output: r.get(4)?,
            reasoning_tokens: r.get(5)?,
            tokens: r.get(6)?,
            events: r.get(7)?,
            sessions: r.get(8)?,
            subagent_tokens: r.get(9)?,
            first_ts: r.get(10)?,
            last_ts: r.get(11)?,
            measured_tokens: r.get(12)?,
            estimated_tokens: r.get(13)?,
            priced_tokens,
            priced_events: r.get(15)?,
            priced_cost_usd,
            cost_per_mtok,
            ..Default::default()
        })
    })?;

    let mut stmt = conn.prepare(&format!(
        "SELECT u.source, COALESCE(SUM({T}),0), SUM(u.n),
                COUNT(DISTINCT u.session_id),
                SUM(CASE WHEN u.estimated = 0 AND u.cost_usd IS NOT NULL THEN u.cost_usd END)
         FROM {rows} u
         LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
         WHERE {SHOWS_AS_2} AND {H}
         GROUP BY u.source ORDER BY 2 DESC, 1",
        T = TOKENS,
        H = NOT_HIDDEN
    ))?;
    side.by_source = stmt
        .query_map(rusqlite::params![cutoff_ts, model], |r| {
            Ok(SourceTotals {
                source: r.get(0)?,
                tokens: r.get(1)?,
                events: r.get(2)?,
                sessions: r.get(3)?,
                cost_usd: finite(r.get(4)?),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut stmt = conn.prepare(&format!(
        "SELECT {P}, COALESCE(SUM({T}),0), SUM(u.n),
                COUNT(DISTINCT u.source || char(31) || u.session_id),
                SUM(CASE WHEN u.estimated = 0 AND u.cost_usd IS NOT NULL THEN u.cost_usd END),
                MIN(u.min_ts), MAX(u.max_ts)
         FROM {rows} u
         LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
         LEFT JOIN project_alias pj ON pj.alias = u.project
         WHERE {SHOWS_AS_2} AND {H}
         GROUP BY 1 ORDER BY 2 DESC, 1",
        P = PROJECT,
        T = TOKENS,
        H = NOT_HIDDEN
    ))?;
    side.by_project = stmt
        .query_map(rusqlite::params![cutoff_ts, model], |r| {
            Ok(ProjectRow {
                project: r.get(0)?,
                tokens: r.get(1)?,
                events: r.get(2)?,
                sessions: r.get(3)?,
                cost_usd: finite(r.get(4)?),
                first_ts: r.get(5)?,
                last_ts: r.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let daily: Vec<HeatmapCell> = {
        let mut stmt = conn.prepare(&format!(
            "SELECT date(u.ts/1000,'unixepoch') AS d, COALESCE(SUM({T}),0)
             FROM {rows} u
             LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
             WHERE {SHOWS_AS_2} AND {H}
             GROUP BY d ORDER BY d",
            T = TOKENS,
            H = NOT_HIDDEN
        ))?;
        let rows = stmt.query_map(rusqlite::params![cutoff_ts, model], |r| {
            Ok(HeatmapCell { date: r.get(0)?, tokens: r.get(1)? })
        })?
        .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    let dates: Vec<String> = daily.iter().map(|d| d.date.clone()).collect();
    side.active_days = dates.len() as i64;
    (side.current_streak, side.longest_streak) = streaks(&dates);
    let mut peak: Option<&HeatmapCell> = None;
    for day in &daily {
        if peak.is_none_or(|p| day.tokens > p.tokens) {
            peak = Some(day);
        }
    }
    side.peak_day = peak.map(|p| p.date.clone());
    side.peak_day_tokens = peak.map(|p| p.tokens).unwrap_or(0);

    let sparse_hours: Vec<HourRow> = {
        let mut stmt = conn.prepare(&format!(
            "SELECT CAST(strftime('%H', u.ts/1000, 'unixepoch') AS INTEGER), COALESCE(SUM({T}),0)
             FROM {rows} u
             LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
             WHERE {SHOWS_AS_2} AND {H}
             GROUP BY 1 ORDER BY 1",
            T = TOKENS,
            H = NOT_HIDDEN
        ))?;
        let rows = stmt.query_map(rusqlite::params![cutoff_ts, model], |r| {
            Ok(HourRow { hour: r.get(0)?, tokens: r.get(1)? })
        })?
        .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    side.hourly = (0..24)
        .map(|hour| HourRow {
            hour,
            tokens: sparse_hours.iter().find(|h| h.hour == hour).map(|h| h.tokens).unwrap_or(0),
        })
        .collect();
    let mut busiest: Option<&HourRow> = None;
    for hour in &side.hourly {
        if hour.tokens > 0 && busiest.is_none_or(|b| hour.tokens > b.tokens) {
            busiest = Some(hour);
        }
    }
    side.busiest_hour = busiest.map(|h| h.hour);
    let night_tokens: i64 = side
        .hourly
        .iter()
        .filter(|h| h.hour >= 22 || h.hour < 6)
        .map(|h| h.tokens)
        .sum();
    side.night_share = if side.tokens > 0 {
        night_tokens as f64 / side.tokens as f64
    } else {
        0.0
    };
    side.timing = comparison_timing(conn, model, cutoff_ts)?;
    Ok((side, daily))
}

fn comparison_rivalry(daily: &[ComparisonDailyRow]) -> Rivalry {
    let mut rivalry = Rivalry::default();
    let mut cumulative_a = 0i64;
    let mut cumulative_b = 0i64;
    let mut leader: Option<String> = None;

    for day in daily {
        match day.a_tokens.cmp(&day.b_tokens) {
            std::cmp::Ordering::Greater => rivalry.a_wins += 1,
            std::cmp::Ordering::Less => rivalry.b_wins += 1,
            std::cmp::Ordering::Equal => rivalry.ties += 1,
        }
        let margin = (day.a_tokens - day.b_tokens).abs();
        if margin > 0 && rivalry.biggest_gap.as_ref().is_none_or(|gap| margin > gap.margin) {
            rivalry.biggest_gap = Some(DailyGap {
                date: day.date.clone(),
                wider_side: if day.a_tokens > day.b_tokens { "a" } else { "b" }.to_string(),
                margin,
            });
        }

        cumulative_a += day.a_tokens;
        cumulative_b += day.b_tokens;
        if cumulative_a == cumulative_b {
            continue;
        }
        let direction = if cumulative_a > cumulative_b { "a" } else { "b" }.to_string();
        if let Some(previous) = &leader {
            if previous != &direction {
                rivalry.lead_changes.push(LeadChange {
                    date: day.date.clone(),
                    direction: direction.clone(),
                });
            }
        }
        rivalry.latest_date = Some(day.date.clone());
        rivalry.latest_direction = Some(direction.clone());
        leader = Some(direction);
    }
    rivalry
}

/// Compare two canonical display names in one trailing window. `None` means
/// that a name is unknown, hidden, or the inputs are not two distinct models.
pub fn model_comparison(
    store: &Store,
    model_a: &str,
    model_b: &str,
    days: i64,
    anchor_tokens: i64,
) -> DbResult<Option<ModelComparison>> {
    let model_a = model_a.trim();
    let model_b = model_b.trim();
    if model_a.is_empty() || model_b.is_empty() || model_a == model_b || anchor_tokens <= 0 {
        return Ok(None);
    }
    let conn = store.read_conn();
    let known: HashSet<String> = {
        let mut stmt = conn.prepare(&format!(
            "SELECT DISTINCT COALESCE(a.canonical, u.model, 'unknown')
             FROM {R} u
             LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
             WHERE COALESCE(a.canonical, u.model, 'unknown') IN (?1, ?2) AND {H}",
            H = NOT_HIDDEN,
            R = usage_rows_where(None, &|row| {
                let (one, two) = (model_rows(1)(row), model_rows(2)(row));
                format!("({one} OR {two})")
            })
        ))?;
        let rows = stmt.query_map(rusqlite::params![model_a, model_b], |r| r.get(0))?
            .collect::<Result<HashSet<_>, _>>()?;
        rows
    };
    if known.len() != 2 || !known.contains(model_a) || !known.contains(model_b) {
        return Ok(None);
    }

    let cutoff_ts = cutoff(days);
    let (a, daily_a) = comparison_side(&conn, model_a, cutoff_ts)?;
    let (b, daily_b) = comparison_side(&conn, model_b, cutoff_ts)?;
    let mut by_date: BTreeMap<String, [i64; 2]> = BTreeMap::new();
    for day in daily_a {
        by_date.entry(day.date).or_default()[0] = day.tokens;
    }
    for day in daily_b {
        by_date.entry(day.date).or_default()[1] = day.tokens;
    }
    let daily: Vec<ComparisonDailyRow> = by_date
        .into_iter()
        .filter(|(_, tokens)| tokens[0] > 0 || tokens[1] > 0)
        .map(|(date, tokens)| ComparisonDailyRow {
            date,
            a_tokens: tokens[0],
            b_tokens: tokens[1],
        })
        .collect();
    let rivalry = comparison_rivalry(&daily);
    let total_window_tokens = window_tokens(&conn, cutoff_ts)?;

    let rate_a = blended_rate(a.priced_tokens, a.priced_cost_usd);
    let rate_b = blended_rate(b.priced_tokens, b.priced_cost_usd);
    let cost_comparison = match (rate_a, rate_b) {
        (Some(rate_a), Some(rate_b)) => {
            let a_anchor_cost_usd = anchor_tokens as f64 * rate_a;
            let b_anchor_cost_usd = anchor_tokens as f64 * rate_b;
            let b_equivalent_tokens = if rate_a > 0.0 && rate_b > 0.0 {
                finite(Some(anchor_tokens as f64 * (rate_a / rate_b)))
            } else {
                None
            };
            match (finite(Some(a_anchor_cost_usd)), finite(Some(b_anchor_cost_usd))) {
                (Some(a_anchor_cost_usd), Some(b_anchor_cost_usd)) => Some(CostComparison {
                    anchor_tokens,
                    a_anchor_cost_usd,
                    b_anchor_cost_usd,
                    b_equivalent_tokens,
                }),
                _ => None,
            }
        }
        _ => None,
    };

    Ok(Some(ModelComparison {
        a,
        b,
        daily,
        rivalry,
        total_window_tokens,
        cost_comparison,
    }))
}

fn format_tokens(n: i64) -> String {
    if n >= 1_000_000_000 {
        if n % 1_000_000_000 == 0 || n >= 10_000_000_000 {
            format!("{}B", n / 1_000_000_000)
        } else {
            format!("{:.1}B", n as f64 / 1e9)
        }
    } else if n >= 1_000_000 {
        if n % 1_000_000 == 0 || n >= 10_000_000 {
            format!("{}M", n / 1_000_000)
        } else {
            format!("{:.1}M", n as f64 / 1e6)
        }
    } else if n >= 1_000 {
        if n % 1_000 == 0 || n >= 10_000 {
            format!("{}K", n / 1_000)
        } else {
            format!("{:.1}K", n as f64 / 1e3)
        }
    } else {
        n.to_string()
    }
}

/// Token Milestones and Big Spender tiers for the rows `shows_as` picks out
/// (`narrow` is its `usage_rows_where` narrowing; `params` bind both), each
/// stamped with the call that crossed it — what a walk over every event in
/// time order would find, without walking every event. Hourly running totals
/// come from the rollup; only an hour in which a tier is crossed is read event
/// by event, in order, to find the crossing call. Tokens and costs never go
/// down, so a tier crossed inside an hour is crossed by that hour's total too.
fn milestone_achievements(
    conn: &rusqlite::Connection,
    shows_as: &str,
    narrow: &dyn Fn(&str) -> String,
    params: &[rusqlite::types::Value],
) -> DbResult<Vec<Achievement>> {
    const TOKEN_TIERS: [(i64, &str, &str, &str); 3] = [
        (100_000_000, "100m", "100M Tokens", "100M tokens"),
        (500_000_000, "500m", "500M Tokens", "500M tokens"),
        (1_000_000_000, "1b", "1B Tokens", "1B tokens"),
    ];
    const COST_TIERS: [(f64, &str, &str, &str); 3] = [
        (100.0, "100", "$100 Spent", "$100 spent"),
        (500.0, "500", "$500 Spent", "$500 spent"),
        (1000.0, "1k", "$1K Spent", "$1,000 spent"),
    ];
    use rusqlite::types::Value;

    // `ts / 3600000` truncates, like the rollup's hour, and orders the same way
    // `ts` does — including the odd row before 1970, which reads as events.
    let hours: Vec<(i64, i64, f64)> = {
        let mut stmt = conn.prepare(&format!(
            "SELECT u.ts / 3600000 AS h, COALESCE(SUM({T}),0), TOTAL(u.cost_usd)
             FROM {R} u
             LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
             WHERE {shows_as} AND {H}
             GROUP BY h ORDER BY h",
            T = TOKENS,
            H = NOT_HIDDEN,
            R = usage_rows_where(None, narrow)
        ))?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(params.iter()), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    let k = params.len();
    let mut hour_events = conn.prepare(&format!(
        "SELECT u.ts, {T}, COALESCE(u.cost_usd, 0.0)
         FROM usage_event u
         LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
         WHERE u.ts > ?{lo} AND u.ts < ?{hi} AND u.ts / 3600000 = ?{h}
           AND {shows_as} AND {N} AND {H}
         ORDER BY u.ts ASC",
        T = TOKENS,
        H = NOT_HIDDEN,
        N = narrow("u"),
        lo = k + 1,
        hi = k + 2,
        h = k + 3
    ))?;

    let (mut tokens, mut cost) = (0i64, 0.0f64);
    let (mut next_token, mut next_cost) = (0, 0);
    let (mut earned_tokens, mut earned_cost) = (Vec::new(), Vec::new());
    for (h, hour_tokens, hour_cost) in hours {
        if next_token >= TOKEN_TIERS.len() && next_cost >= COST_TIERS.len() {
            break;
        }
        let crosses = TOKEN_TIERS.get(next_token).is_some_and(|t| tokens + hour_tokens >= t.0)
            || COST_TIERS.get(next_cost).is_some_and(|t| cost + hour_cost >= t.0);
        if !crosses {
            tokens += hour_tokens;
            cost += hour_cost;
            continue;
        }
        let mut bind = params.to_vec();
        bind.extend([
            Value::Integer((h - 1) * 3_600_000),
            Value::Integer((h + 1) * 3_600_000),
            Value::Integer(h),
        ]);
        let mut rows = hour_events.query(rusqlite::params_from_iter(bind.iter()))?;
        while let Some(row) = rows.next()? {
            let ts: i64 = row.get(0)?;
            tokens += row.get::<_, i64>(1)?;
            cost += row.get::<_, f64>(2)?;
            while let Some(&(_, tier, title, value)) = TOKEN_TIERS.get(next_token).filter(|t| tokens >= t.0) {
                earned_tokens.push(Achievement {
                    kind: "token_milestone".to_string(),
                    tier: Some(tier.to_string()),
                    title: title.to_string(),
                    value: value.to_string(),
                    earned_ts: Some(ts),
                });
                next_token += 1;
            }
            while let Some(&(_, tier, title, value)) = COST_TIERS.get(next_cost).filter(|t| cost >= t.0) {
                earned_cost.push(Achievement {
                    kind: "big_spender".to_string(),
                    tier: Some(tier.to_string()),
                    title: title.to_string(),
                    value: value.to_string(),
                    earned_ts: Some(ts),
                });
                next_cost += 1;
            }
        }
    }
    earned_tokens.extend(earned_cost);
    Ok(earned_tokens)
}

pub fn model_achievements(store: &Store, model: &str) -> DbResult<Vec<Achievement>> {
    let exists_sql = format!(
        "SELECT COALESCE(a.canonical, u.model, 'unknown')
         FROM usage_event u LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
         WHERE {MODEL_IS_1} AND {H}
         LIMIT 1",
        H = NOT_HIDDEN
    );
    // Before `read_conn`: see `overview`.
    let standings = daily_standings(store)?;
    let conn = store.read_conn();
    let mut exists_stmt = conn.prepare(&exists_sql)?;
    let canonical_name = match exists_stmt.query_row(rusqlite::params![model], |r| r.get::<_, String>(0)) {
        Ok(name) => name,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };

    let mut achievements = Vec::new();
    let rows = usage_rows_where(None, &model_rows(1));

    // 1. Daily usage for Peak Day and Longest Streak
    let sql_daily = format!(
        "SELECT date(ts/1000, 'unixepoch') AS d, COALESCE(SUM({T}), 0), MIN(u.min_ts)
         FROM {rows} u LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
         WHERE {SHOWS_AS_1} AND {H}
         GROUP BY d ORDER BY d",
        T = TOKENS,
        H = NOT_HIDDEN
    );
    let mut stmt_daily = conn.prepare(&sql_daily)?;
    let daily_rows: Vec<(String, i64, i64)> = stmt_daily
        .query_map(rusqlite::params![&canonical_name], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    // Peak Day
    if let Some((_d, tokens, min_ts)) = daily_rows.iter().max_by_key(|r| r.1) {
        if *tokens > 0 {
            achievements.push(Achievement {
                kind: "peak_day".to_string(),
                tier: None,
                title: "Peak Day".to_string(),
                value: format!("{} tokens", format_tokens(*tokens)),
                earned_ts: Some(*min_ts),
            });
        }
    }

    // Longest Streak
    let day_fmt = format_description!("[year]-[month]-[day]");
    let mut longest = 0i64;
    let mut longest_end_ts: Option<i64> = None;
    let mut run = 0i64;
    let mut prev: Option<time::Date> = None;

    for (d_str, _tokens, min_ts) in &daily_rows {
        let Ok(parsed) = time::Date::parse(d_str, &day_fmt) else { continue };
        if let Some(p) = prev {
            if next_day(p) == parsed {
                run += 1;
            } else {
                run = 1;
            }
        } else {
            run = 1;
        }
        if run > longest {
            longest = run;
            longest_end_ts = Some(*min_ts);
        }
        prev = Some(parsed);
    }

    if longest >= 2 {
        achievements.push(Achievement {
            kind: "longest_streak".to_string(),
            tier: None,
            title: "Longest Streak".to_string(),
            value: format!("{} days in a row", longest),
            earned_ts: longest_end_ts,
        });
    }

    // 2. Multi-Harness
    let sql_sources = format!(
        "SELECT u.source, MIN(u.min_ts) as first_ts
         FROM {rows} u LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
         WHERE {SHOWS_AS_1} AND {H}
         GROUP BY u.source
         ORDER BY first_ts ASC",
        H = NOT_HIDDEN
    );
    let mut stmt_sources = conn.prepare(&sql_sources)?;
    let sources: Vec<(String, i64)> = stmt_sources
        .query_map(rusqlite::params![&canonical_name], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    if sources.len() >= 2 {
        achievements.push(Achievement {
            kind: "multi_harness".to_string(),
            tier: None,
            title: "Multi-Harness".to_string(),
            value: format!("Used in {} harnesses", sources.len()),
            earned_ts: Some(sources[1].1),
        });
    }

    // 3. Family Champion. All-time totals per model come from the memoised
    // standings rather than another pass over every event.
    let fam = families::family_for(&canonical_name);
    if fam != "Other" {
        // The family's highest total; a tie goes to the first name, matching the
        // standings' own (tokens desc, name asc) order.
        let top_in_fam = standings
            .names
            .iter()
            .zip(standings.totals())
            .filter(|(m, _)| families::family_for(m) == fam)
            .min_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));

        if let Some((top_model, top_tokens)) = top_in_fam {
            if *top_model == canonical_name && top_tokens > 0 {
                achievements.push(Achievement {
                    kind: "family_champion".to_string(),
                    tier: None,
                    title: "Family Champion".to_string(),
                    value: format!("Top model in {}", fam),
                    earned_ts: None,
                });
            }
        }
    }

    // 4. Token Milestones & Big Spender
    achievements.extend(milestone_achievements(
        &conn,
        SHOWS_AS_1,
        &model_rows(1),
        &[rusqlite::types::Value::Text(canonical_name.clone())],
    )?);

    // 5. First Project & Project Explorer
    let sql_projects = format!(
        "SELECT {P} as p, MIN(u.min_ts) as first_ts
         FROM {rows} u
         LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
         LEFT JOIN project_alias pj ON pj.alias = u.project
         WHERE {SHOWS_AS_1} AND {H}
           AND u.project IS NOT NULL AND u.project != '' AND u.project != 'unknown'
           AND {P} != 'unknown'
         GROUP BY p
         ORDER BY first_ts ASC, p ASC",
        H = NOT_HIDDEN,
        P = PROJECT
    );
    let mut stmt_projects = conn.prepare(&sql_projects)?;
    let projects: Vec<(String, i64)> = stmt_projects
        .query_map(rusqlite::params![&canonical_name], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    if let Some((first_proj, first_ts)) = projects.first() {
        achievements.push(Achievement {
            kind: "first_project".to_string(),
            tier: None,
            title: "First Project".to_string(),
            value: first_proj.clone(),
            earned_ts: Some(*first_ts),
        });
    }

    if projects.len() >= 3 {
        achievements.push(Achievement {
            kind: "project_explorer".to_string(),
            tier: None,
            title: "Project Explorer".to_string(),
            value: format!("Used in {} projects", projects.len()),
            earned_ts: Some(projects[2].1),
        });
    }

    Ok(achievements)
}

/// Group model stats into families.  Runs `model_stats` under the hood and
/// assigns each display-name row to a family via the pricing prefix table.
/// Sessions are summed across member models (a session using two models of
/// one family will count twice — acceptable for a summary).
pub fn family_stats(store: &Store, days: i64) -> DbResult<Vec<FamilyStatsRow>> {
    let rows = model_stats(store, days)?;
    let mut map: HashMap<String, FamilyStatsRow> = HashMap::new();

    for r in rows {
        let fam = families::family_for(&r.model);
        let e = map
            .entry(fam.to_string())
            .or_insert_with(|| FamilyStatsRow {
                family: fam.to_string(),
                tokens: 0,
                input_tokens: 0,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                events: 0,
                sessions: 0,
                cost_usd: None,
                first_ts: None,
                last_ts: None,
                sources: Vec::new(),
                models: Vec::new(),
            });

        e.tokens += r.tokens;
        e.input_tokens += r.input_tokens;
        e.output_tokens += r.output_tokens;
        e.cache_read_tokens += r.cache_read_tokens;
        e.cache_write_tokens += r.cache_write_tokens;
        e.events += r.events;
        e.sessions += r.sessions;

        // Merge costs: sum present values; keep None only when every member is unpriced.
        match (e.cost_usd, r.cost_usd) {
            (Some(a), Some(b)) => e.cost_usd = Some(a + b),
            (None, Some(b)) => e.cost_usd = Some(b),
            (Some(_), None) => {}
            (None, None) => {}
        }

        // Earliest / latest timestamps.
        match (e.first_ts, r.first_ts) {
            (Some(a), Some(b)) => e.first_ts = Some(a.min(b)),
            (None, o) => e.first_ts = o,
            _ => {}
        }
        match (e.last_ts, r.last_ts) {
            (Some(a), Some(b)) => e.last_ts = Some(a.max(b)),
            (None, o) => e.last_ts = o,
            _ => {}
        }

        // Union sources.
        for s in &r.sources {
            if !e.sources.contains(s) {
                e.sources.push(s.clone());
            }
        }

        e.models.push(r);
    }

    let mut families: Vec<FamilyStatsRow> = map.into_values().collect();
    families.sort_by_key(|b| std::cmp::Reverse(b.tokens));
    for f in &mut families {
        f.models.sort_by_key(|b| std::cmp::Reverse(b.tokens));
    }
    Ok(families)
}

/// The display names of every model that belongs to `family`.
///
/// `families::family_for` is the one place the prefix rules live and SQLite
/// cannot call it, so membership is resolved here and the queries below filter
/// on the resulting names. Hidden models are already absent from the list.
///
/// The visible display names are exactly the standings' model list, which is
/// memoised, so a family page no longer scans every event twice to find them.
fn family_model_names(store: &Store, family: &str) -> DbResult<Vec<String>> {
    Ok(daily_standings(store)?
        .names
        .iter()
        .filter(|m| families::family_for(m) == family)
        .cloned()
        .collect())
}

/// Raw model names that can show as one of `names`: each name itself plus every
/// alias with a segment shown as one of them. Read from `model_alias` alone, so
/// it costs nothing next to the event queries it narrows.
fn family_raw_names(conn: &rusqlite::Connection, names: &[String]) -> DbResult<Vec<String>> {
    let wanted: HashSet<&str> = names.iter().map(String::as_str).collect();
    let mut raw: BTreeSet<String> = names.iter().cloned().collect();
    let mut stmt = conn.prepare("SELECT alias, canonical FROM model_alias")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    for row in rows {
        let (alias, canonical) = row?;
        if wanted.contains(canonical.as_str()) {
            raw.insert(alias);
        }
    }
    Ok(raw.into_iter().collect())
}

/// `?first, ?first+1, …`: `n` numbered holes. Numbered rather than bare `?`,
/// which bind by position in the text and so cannot appear twice.
fn holes(first: usize, n: usize) -> String {
    (first..first + n).map(|i| format!("?{i}")).collect::<Vec<_>>().join(",")
}

/// "This row displays as a family member", for the member names bound from
/// `?first` on (`family_name_params`' order). Needs the `a` alias join. The
/// exact test; `family_rows` is its narrowing.
fn family_shows_as(names: &[String], first: usize) -> String {
    format!("COALESCE(a.canonical, u.model, 'unknown') IN ({})", holes(first, names.len()))
}

/// The `usage_rows_where` narrowing for `family_shows_as`: rows whose raw name
/// is one of `family_raw_names` (bound right after the member names), plus
/// NULL models, which show as 'unknown'. As with `MODEL_IS_1`, it only lets
/// SQLite skip rows that cannot match.
fn family_rows(names: &[String], raw: &[String], first: usize) -> impl Fn(&str) -> String {
    let raw_holes = holes(first + names.len(), raw.len());
    move |row| format!("({row}.model IN ({raw_holes}) OR {row}.model IS NULL)")
}

/// Values for the family holes: member names, then raw names.
fn family_name_params(names: &[String], raw: &[String]) -> Vec<rusqlite::types::Value> {
    names
        .iter()
        .chain(raw)
        .cloned()
        .map(rusqlite::types::Value::Text)
        .collect()
}

/// Full detail card for one model family, keyed by the family label (matched
/// case-insensitively) and filtered by range in days. Returns `None` when no
/// visible model has ever resolved to that family.
pub fn family_detail(store: &Store, family: &str, days: i64) -> DbResult<Option<FamilyDetail>> {
    let Some(fam) = families::canonical_family_name(family) else {
        return Ok(None);
    };
    // Before `read_conn`: see `overview`.
    let names = family_model_names(store, fam)?;
    let conn = store.read_conn();
    if names.is_empty() {
        return Ok(None);
    }

    let cutoff_ts = cutoff(days);
    let raw = family_raw_names(&conn, &names)?;
    // ?1 is the window start; the names follow it.
    let filter = family_shows_as(&names, 2);
    let rows = usage_rows_where(Some("?1"), &family_rows(&names, &raw, 2));
    let mut params = vec![rusqlite::types::Value::Integer(cutoff_ts)];
    params.extend(family_name_params(&names, &raw));

    let totals_sql = format!(
        "SELECT COALESCE(SUM(u.input_tokens),0),
                COALESCE(SUM(CASE WHEN u.source = 'antigravity' THEN u.output_tokens + COALESCE(u.reasoning_tokens,0) ELSE u.output_tokens END),0),
                COALESCE(SUM(u.cache_read_tokens),0), COALESCE(SUM(u.cache_write_tokens),0),
                COALESCE(SUM(u.reasoning_tokens),0),
                COALESCE(SUM({T}),0), COALESCE(SUM(u.n),0), COUNT(DISTINCT u.session_id),
                SUM(u.cost_usd), MIN(u.min_ts), MAX(u.max_ts)
         FROM {rows} u
         LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
         WHERE {F} AND {H}",
        T = TOKENS,
        F = filter,
        H = NOT_HIDDEN
    );
    let mut stmt = conn.prepare(&totals_sql)?;
    let (inp, out, cr, cw, reasoning, total, events, sessions, cost, first_ts, last_ts) =
        stmt.query_row(rusqlite::params_from_iter(params.iter()), |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, i64>(7)?,
                r.get::<_, Option<f64>>(8)?,
                r.get::<_, Option<i64>>(9)?,
                r.get::<_, Option<i64>>(10)?,
            ))
        })?;

    // Same shape as `model_stats`, restricted to the family's display names.
    let models_sql = format!(
        "SELECT COALESCE(a.canonical, u.model, 'unknown'),
                COALESCE(SUM(u.input_tokens),0), COALESCE(SUM(u.output_tokens),0),
                COALESCE(SUM(u.cache_read_tokens),0), COALESCE(SUM(u.cache_write_tokens),0),
                COALESCE(SUM({T}),0), SUM(u.n), COUNT(DISTINCT u.session_id),
                SUM(u.cost_usd), MIN(u.min_ts), MAX(u.max_ts), GROUP_CONCAT(DISTINCT u.source),
                COUNT(DISTINCT date(u.ts/1000, 'unixepoch'))
         FROM {rows} u
         LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
         WHERE {F} AND {H}
         GROUP BY 1 ORDER BY 6 DESC",
        T = TOKENS,
        F = filter,
        H = NOT_HIDDEN
    );
    let mut stmt = conn.prepare(&models_sql)?;
    let models: Vec<ModelStatsRow> = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |r| {
            let src_str: Option<String> = r.get(11)?;
            let sources = src_str
                .map(|s| s.split(',').map(String::from).collect())
                .unwrap_or_default();
            Ok(ModelStatsRow {
                model: r.get(0)?,
                input_tokens: r.get(1)?,
                output_tokens: r.get(2)?,
                cache_read_tokens: r.get(3)?,
                cache_write_tokens: r.get(4)?,
                tokens: r.get(5)?,
                events: r.get(6)?,
                sessions: r.get(7)?,
                cost_usd: r.get(8)?,
                first_ts: r.get(9)?,
                last_ts: r.get(10)?,
                sources,
                active_days: r.get(12)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let by_source = {
        let mut stmt = conn.prepare(&format!(
            "SELECT u.source, COALESCE(SUM({T}),0), SUM(u.n), COUNT(DISTINCT u.session_id), SUM(u.cost_usd)
             FROM {rows} u
             LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
             WHERE {F} AND {H}
             GROUP BY u.source ORDER BY 2 DESC",
            T = TOKENS,
            F = filter,
            H = NOT_HIDDEN
        ))?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(params.iter()), |r| {
                Ok(SourceTotals {
                    source: r.get(0)?,
                    tokens: r.get(1)?,
                    events: r.get(2)?,
                    sessions: r.get(3)?,
                    cost_usd: r.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };

    let by_project = {
        let mut stmt = conn.prepare(&format!(
            "SELECT {P}, COALESCE(SUM({T}),0), SUM(u.n),
                    COUNT(DISTINCT u.session_id), SUM(u.cost_usd), MIN(u.min_ts), MAX(u.max_ts)
             FROM {rows} u
             LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
             LEFT JOIN project_alias pj ON pj.alias = u.project
             WHERE {F} AND {H}
             GROUP BY 1 ORDER BY 2 DESC",
            P = PROJECT,
            T = TOKENS,
            F = filter,
            H = NOT_HIDDEN
        ))?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(params.iter()), |r| {
                Ok(ProjectRow {
                    project: r.get(0)?,
                    tokens: r.get(1)?,
                    events: r.get(2)?,
                    sessions: r.get(3)?,
                    cost_usd: r.get(4)?,
                    first_ts: r.get(5)?,
                    last_ts: r.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };

    let daily: Vec<HeatmapCell> = {
        let mut stmt = conn.prepare(&format!(
            "SELECT date(ts/1000,'unixepoch') AS d, COALESCE(SUM({T}),0)
             FROM {rows} u
             LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
             WHERE {F} AND {H}
             GROUP BY d ORDER BY d",
            T = TOKENS,
            F = filter,
            H = NOT_HIDDEN
        ))?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(params.iter()), |r| {
                Ok(HeatmapCell { date: r.get(0)?, tokens: r.get(1)? })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };

    let dates: Vec<String> = daily.iter().map(|d| d.date.clone()).collect();
    let active_days = dates.len() as i64;
    let (current_streak, longest_streak) = streaks(&dates);
    let peak = daily.iter().max_by_key(|d| d.tokens);

    let total_window_tokens = window_tokens(&conn, cutoff_ts)?;

    Ok(Some(FamilyDetail {
        family: fam.to_string(),
        tokens: total,
        input_tokens: inp,
        output_tokens: out,
        reasoning_tokens: reasoning,
        cache_read_tokens: cr,
        cache_write_tokens: cw,
        events,
        sessions,
        cost_usd: cost,
        first_ts,
        last_ts,
        active_days,
        current_streak,
        longest_streak,
        peak_day: peak.map(|p| p.date.clone()),
        peak_day_tokens: peak.map(|p| p.tokens).unwrap_or(0),
        models,
        by_source,
        by_project,
        daily,
        total_window_tokens,
    }))
}

/// Achievements for a whole family: the same milestones one model can earn,
/// accumulated across every model that belongs to it. All-time by design — an
/// achievement is something that happened, not something inside a range.
pub fn family_achievements(store: &Store, family: &str) -> DbResult<Vec<Achievement>> {
    let Some(fam) = families::canonical_family_name(family) else {
        return Ok(Vec::new());
    };
    // Before `read_conn`: see `overview`.
    let standings = daily_standings(store)?;
    let names = family_model_names(store, fam)?;
    let conn = store.read_conn();
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let raw = family_raw_names(&conn, &names)?;
    let filter = family_shows_as(&names, 1);
    let narrow = family_rows(&names, &raw, 1);
    let rows = usage_rows_where(None, &narrow);
    let name_values = family_name_params(&names, &raw);

    let mut achievements = Vec::new();

    // 1. Daily usage for Peak Day and Longest Streak
    let daily_rows: Vec<(String, i64, i64)> = {
        let mut stmt = conn.prepare(&format!(
            "SELECT date(ts/1000, 'unixepoch') AS d, COALESCE(SUM({T}), 0), MIN(u.min_ts)
             FROM {rows} u
             LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
             WHERE {F} AND {H}
             GROUP BY d ORDER BY d",
            T = TOKENS,
            F = filter,
            H = NOT_HIDDEN
        ))?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(name_values.iter()), |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };

    if let Some((_d, tokens, min_ts)) = daily_rows.iter().max_by_key(|r| r.1) {
        if *tokens > 0 {
            achievements.push(Achievement {
                kind: "peak_day".to_string(),
                tier: None,
                title: "Peak Day".to_string(),
                value: format!("{} tokens", format_tokens(*tokens)),
                earned_ts: Some(*min_ts),
            });
        }
    }

    let day_fmt = format_description!("[year]-[month]-[day]");
    let mut longest = 0i64;
    let mut longest_end_ts: Option<i64> = None;
    let mut run = 0i64;
    let mut prev: Option<time::Date> = None;

    for (d_str, _tokens, min_ts) in &daily_rows {
        let Ok(parsed) = time::Date::parse(d_str, &day_fmt) else {
            continue;
        };
        if let Some(p) = prev {
            if next_day(p) == parsed {
                run += 1;
            } else {
                run = 1;
            }
        } else {
            run = 1;
        }
        if run > longest {
            longest = run;
            longest_end_ts = Some(*min_ts);
        }
        prev = Some(parsed);
    }

    if longest >= 2 {
        achievements.push(Achievement {
            kind: "longest_streak".to_string(),
            tier: None,
            title: "Longest Streak".to_string(),
            value: format!("{} days in a row", longest),
            earned_ts: longest_end_ts,
        });
    }

    // 2. Multi-Harness
    let sources: Vec<(String, i64)> = {
        let mut stmt = conn.prepare(&format!(
            "SELECT u.source, MIN(u.min_ts) AS first_ts
             FROM {rows} u
             LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
             WHERE {F} AND {H}
             GROUP BY u.source ORDER BY first_ts ASC",
            F = filter,
            H = NOT_HIDDEN
        ))?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(name_values.iter()), |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };

    if sources.len() >= 2 {
        achievements.push(Achievement {
            kind: "multi_harness".to_string(),
            tier: None,
            title: "Multi-Harness".to_string(),
            value: format!("Used in {} harnesses", sources.len()),
            earned_ts: Some(sources[1].1),
        });
    }

    // 3. Family Champion — the family that owns the most tokens overall
    // carries the crown. "Other" is a bucket, not a family, so it never does.
    if fam != "Other" {
        // All-time totals per model come from the memoised standings.
        let family_totals: HashMap<&'static str, i64> = {
            let mut totals: HashMap<&'static str, i64> = HashMap::new();
            for (model, tokens) in standings.names.iter().zip(standings.totals()) {
                *totals.entry(families::family_for(model)).or_insert(0) += tokens;
            }
            totals
        };
        let top = family_totals.iter().max_by_key(|(_, tokens)| **tokens);
        if let Some((top_family, top_tokens)) = top {
            if *top_family == fam && *top_tokens > 0 {
                achievements.push(Achievement {
                    kind: "family_champion".to_string(),
                    tier: None,
                    title: "Top Family".to_string(),
                    value: "Most used model family".to_string(),
                    earned_ts: None,
                });
            }
        }
    }

    // 4. Token Milestones & Big Spender
    achievements.extend(milestone_achievements(&conn, &filter, &narrow, &name_values)?);

    // 5. First Project & Project Explorer
    let projects: Vec<(String, i64)> = {
        let mut stmt = conn.prepare(&format!(
            "SELECT {P} AS p, MIN(u.min_ts) AS first_ts
             FROM {rows} u
             LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
             LEFT JOIN project_alias pj ON pj.alias = u.project
             WHERE {F} AND {H}
               AND u.project IS NOT NULL AND u.project != '' AND u.project != 'unknown'
               AND {P} != 'unknown'
             GROUP BY p
             ORDER BY first_ts ASC, p ASC",
            F = filter,
            H = NOT_HIDDEN,
            P = PROJECT
        ))?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(name_values.iter()), |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };

    if let Some((first_proj, first_ts)) = projects.first() {
        achievements.push(Achievement {
            kind: "first_project".to_string(),
            tier: None,
            title: "First Project".to_string(),
            value: first_proj.clone(),
            earned_ts: Some(*first_ts),
        });
    }

    if projects.len() >= 3 {
        achievements.push(Achievement {
            kind: "project_explorer".to_string(),
            tier: None,
            title: "Project Explorer".to_string(),
            value: format!("Used in {} projects", projects.len()),
            earned_ts: Some(projects[2].1),
        });
    }

    Ok(achievements)
}

pub fn by_project(store: &Store, days: i64) -> DbResult<Vec<ProjectRow>> {
    let sql = format!(
        "SELECT {P}, COALESCE(SUM({T}),0), SUM(u.n),
                COUNT(DISTINCT session_id), SUM(cost_usd), MIN(u.min_ts), MAX(u.max_ts)
         FROM {R} u
         LEFT JOIN project_alias pj ON pj.alias = u.project
         WHERE {H} GROUP BY 1 ORDER BY 2 DESC",
        T = TOKENS,
        H = NOT_HIDDEN,
        P = PROJECT,
        R = usage_rows(Some("?1"))
    );
    let conn = store.read_conn();
    let mut stmt = conn.prepare(&sql)?;
    let rows: Vec<ProjectRow> = stmt
        .query_map([cutoff(days)], |r| {
            Ok(ProjectRow {
                project: r.get(0)?,
                tokens: r.get(1)?,
                events: r.get(2)?,
                sessions: r.get(3)?,
                cost_usd: r.get(4)?,
                first_ts: r.get(5)?,
                last_ts: r.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// The folder above `path`, if any.
fn folder_parent(path: &str) -> Option<String> {
    let trimmed = path.trim_end_matches('/');
    let i = trimmed.rfind('/')?;
    (i > 0).then(|| trimmed[..i].to_string())
}

/// The last path segment, without a trailing slash.
fn folder_name(path: &str) -> Option<&str> {
    let trimmed = path.trim_end_matches('/');
    let name = match trimmed.rfind('/') {
        Some(i) => &trimmed[i + 1..],
        None => trimmed,
    };
    (!name.is_empty()).then_some(name)
}

/// Projects whose folder no longer exists, paired with a live project that looks
/// like its new home: the same folder name (a move), or a live ancestor (a working
/// directory deep inside a project that was never a repository of its own).
/// Guesswork only: Settings offers it and never applies it, and "keep separate"
/// pins the folder so it stops being suggested.
///
/// Only projects active in the last year are considered, so a long-deleted rename
/// does not nag forever, and the list is capped so a big cleanup cannot flood it.
pub fn project_suggestions(store: &Store) -> DbResult<Vec<ProjectSuggestion>> {
    const MAX_SUGGESTIONS: usize = 20;
    let conn = store.read_conn();
    let sql = format!(
        "SELECT {P}, COALESCE(SUM({T}),0), MAX(u.max_ts)
         FROM {R} u LEFT JOIN project_alias pj ON pj.alias = u.project
         WHERE {P} != 'unknown' GROUP BY 1",
        T = TOKENS,
        P = PROJECT,
        R = usage_rows(None)
    );
    let rows: Vec<(String, i64, Option<i64>)> = {
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    // A self-mapping is a decision, not a guess: the folder was kept on purpose.
    let pinned: std::collections::HashSet<String> = {
        let mut stmt = conn.prepare("SELECT alias FROM project_alias WHERE alias = canonical")?;
        let pinned = stmt.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
        pinned
    };

    let active_since = now_ms() - 365 * 24 * 60 * 60 * 1000;
    let mut live: Vec<(String, String, Option<i64>)> = Vec::new();
    let mut gone: Vec<(String, String, i64, Option<i64>)> = Vec::new();
    for (project, tokens, last) in &rows {
        // Synthetic names are not folders and cannot move.
        if !project.starts_with('/') {
            continue;
        }
        let Some(name) = folder_name(project).map(str::to_string) else {
            continue;
        };
        if std::path::Path::new(project).is_dir() {
            live.push((project.clone(), name, *last));
        } else if !pinned.contains(project) && last.unwrap_or(0) >= active_since {
            gone.push((project.clone(), name, *tokens, *last));
        }
    }
    // A live "project" that contains several other live projects is a container
    // folder — the parent of a checkout tree, where a harness was run once. It is
    // never a merge target for a nested folder, or every deleted repository below it
    // would look like one of its subdirectories.
    let containers: std::collections::HashSet<String> = live
        .iter()
        .filter(|(path, _, _)| {
            let prefix = format!("{path}/");
            live.iter()
                .filter(|(other, _, _)| *other != *path && other.starts_with(&prefix))
                .count()
                >= 2
        })
        .map(|(path, _, _)| path.clone())
        .collect();

    // The most recently used live namesake is the likeliest new home.
    live.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
    gone.sort_by_key(|(_, _, tokens, _)| std::cmp::Reverse(*tokens));

    // Tokens per project, needed for both the dead and the live candidates.
    let stats: std::collections::HashMap<String, (i64, Option<i64>)> = rows
        .iter()
        .map(|(project, tokens, last)| (project.clone(), (*tokens, *last)))
        .collect();

    let mut out: Vec<ProjectSuggestion> = Vec::new();
    for (from, name, tokens, last) in gone {
        // A live project with the same folder name is the likeliest new home.
        let same_name = live
            .iter()
            .find(|(path, n, _)| n == &name && path != &from)
            .map(|(path, _, last)| (path.clone(), *last, "same_name"));
        // Otherwise the dead folder may simply be *inside* a live project: a folder
        // that was never a repository of its own kept turning each new working
        // directory into a project. The deepest live ancestor is the right one.
        let nested = live
            .iter()
            .filter(|(path, _, _)| {
                !containers.contains(path) && from.starts_with(&format!("{path}/"))
            })
            .max_by_key(|(path, _, _)| path.len())
            .map(|(path, _, last)| (path.clone(), *last, "inside"));
        let Some((to, to_last, reason)) = same_name.or(nested) else {
            continue;
        };
        out.push(ProjectSuggestion {
            from,
            to,
            reason: reason.to_string(),
            from_exists: false,
            from_last_ts: last,
            to_last_ts: to_last,
            from_tokens: tokens,
        });
    }

    // The other half of the same story, still live: a folder sitting directly inside
    // a live project that never folded into it, because that project is a plain
    // directory rather than a repository. No repository of its own means no reason
    // for it to be a project, so it is offered — never applied.
    for (path, _, last) in &live {
        if pinned.contains(path) {
            continue;
        }
        let Some(parent) = folder_parent(path) else {
            continue;
        };
        if !live.iter().any(|(p, _, _)| *p == parent) {
            continue;
        }
        if crate::project_id::inspect(path).root.is_some() {
            continue;
        }
        let Some((tokens, _)) = stats.get(path) else {
            continue;
        };
        let parent_last = live
            .iter()
            .find(|(p, _, _)| *p == parent)
            .and_then(|(_, _, l)| *l);
        out.push(ProjectSuggestion {
            from: path.clone(),
            to: parent,
            reason: "subfolder".to_string(),
            from_exists: true,
            from_last_ts: *last,
            to_last_ts: parent_last,
            from_tokens: *tokens,
        });
    }

    // Biggest history first: that is the fold that changes the numbers most.
    out.sort_by_key(|s| std::cmp::Reverse(s.from_tokens));
    out.truncate(MAX_SUGGESTIONS);
    Ok(out)
}

pub fn daily_by_project(store: &Store, days: i64) -> DbResult<Vec<DailyProjectRow>> {
    let sql = format!(
        "SELECT date(ts/1000,'unixepoch') AS d,
                {P},
                COALESCE(SUM({T}),0),
                SUM(cost_usd)
         FROM {R} u
         LEFT JOIN project_alias pj ON pj.alias = u.project
         WHERE {H}
           AND {P} != '' AND {P} != 'unknown'
         GROUP BY d, {P}
         ORDER BY d, {P}",
        T = TOKENS,
        H = NOT_HIDDEN,
        P = PROJECT,
        R = usage_rows(Some("?1"))
    );
    let conn = store.read_conn();
    let mut stmt = conn.prepare(&sql)?;
    let rows: Vec<DailyProjectRow> = stmt
        .query_map([cutoff(days)], |r| {
            Ok(DailyProjectRow {
                date: r.get(0)?,
                project: r.get(1)?,
                tokens: r.get(2)?,
                cost_usd: r.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Full detail card for a single project, keyed by project path (or 'unknown'),
/// filtered by range in days. Returns `None` when the project has no recorded events.
pub fn project_detail(store: &Store, project: &str, days: i64) -> DbResult<Option<ProjectDetail>> {
    // Old links can name a folder that now counts under its project (folded into a
    // root or merged by hand); resolve first so the whole card answers for the
    // project the folder belongs to.
    let project = store.project_canonical(project);
    let is_unknown = project == "unknown";
    let exists_sql = if is_unknown {
        format!(
            "SELECT 1 FROM usage_event u WHERE (u.project IS NULL OR u.project = 'unknown') AND {H} LIMIT 1",
            H = NOT_HIDDEN
        )
    } else {
        format!(
            "SELECT 1 FROM usage_event u
             LEFT JOIN project_alias pj ON pj.alias = u.project
             WHERE {P} = ?1 AND {N} AND {H} LIMIT 1",
            H = NOT_HIDDEN,
            P = PROJECT,
            N = project_rows("u", 1)
        )
    };
    let conn = store.read_conn();
    let mut exists_stmt = conn.prepare(&exists_sql)?;
    let exists_res = if is_unknown {
        exists_stmt.query_row([], |_| Ok(()))
    } else {
        exists_stmt.query_row(rusqlite::params![project], |_| Ok(()))
    };
    match exists_res {
        Ok(_) => {}
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
        Err(e) => return Err(e),
    }

    let cutoff_ts = cutoff(days);
    // 'unknown' is matched on the raw folder, so its filter is its own narrowing.
    let unknown_rows = |row: &str| format!("({row}.project IS NULL OR {row}.project = 'unknown')");
    let named_rows = |row: &str| project_rows(row, 2);
    let (proj_filter, rows) = if is_unknown {
        (unknown_rows("u"), usage_rows_where(Some("?1"), &unknown_rows))
    } else {
        (format!("{PROJECT} = ?2"), usage_rows_where(Some("?1"), &named_rows))
    };

    let sql = format!(
        "SELECT COALESCE(SUM(u.input_tokens),0),
                COALESCE(SUM(CASE WHEN u.source = 'antigravity' THEN u.output_tokens + COALESCE(u.reasoning_tokens,0) ELSE u.output_tokens END),0),
                COALESCE(SUM(u.cache_read_tokens),0), COALESCE(SUM(u.cache_write_tokens),0),
                COALESCE(SUM(u.reasoning_tokens),0),
                COALESCE(SUM({T}),0), COALESCE(SUM(u.n),0), COUNT(DISTINCT u.session_id),
                SUM(u.cost_usd), MIN(u.min_ts), MAX(u.max_ts)
         FROM {rows} u
         LEFT JOIN project_alias pj ON pj.alias = u.project
         WHERE {proj_filter} AND {H}",
        T = TOKENS,
        H = NOT_HIDDEN
    );
    let mut stmt = conn.prepare(&sql)?;
    let map_row = |r: &rusqlite::Row| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, i64>(4)?,
            r.get::<_, i64>(5)?,
            r.get::<_, i64>(6)?,
            r.get::<_, i64>(7)?,
            r.get::<_, Option<f64>>(8)?,
            r.get::<_, Option<i64>>(9)?,
            r.get::<_, Option<i64>>(10)?,
        ))
    };
    let (inp, out, cr, cw, reasoning, total, events, sessions, cost, first_ts, last_ts) = if is_unknown {
        stmt.query_row(rusqlite::params![cutoff_ts], map_row)?
    } else {
        stmt.query_row(rusqlite::params![cutoff_ts, project], map_row)?
    };

    let (inp, out, cr, cw, reasoning, total, events, sessions, cost, first_ts, last_ts) = if events == 0 {
        (0, 0, 0, 0, 0, 0, 0, 0, None, None, None)
    } else {
        (inp, out, cr, cw, reasoning, total, events, sessions, cost, first_ts, last_ts)
    };

    // by_model
    let sql_model = format!(
        "SELECT COALESCE(a.canonical, u.model, 'unknown'),
                COALESCE(SUM({T}),0),
                COALESCE(SUM(u.input_tokens),0),
                COALESCE(SUM(CASE WHEN u.source = 'antigravity' THEN u.output_tokens + COALESCE(u.reasoning_tokens,0) ELSE u.output_tokens END),0),
                SUM(u.n),
                COUNT(DISTINCT u.session_id),
                SUM(u.cost_usd),
                MIN(u.min_ts),
                MAX(u.max_ts)
         FROM {rows} u
         LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
         LEFT JOIN project_alias pj ON pj.alias = u.project
         WHERE {proj_filter} AND {H}
         GROUP BY 1 ORDER BY 2 DESC",
        T = TOKENS,
        H = NOT_HIDDEN
    );
    let mut stmt_model = conn.prepare(&sql_model)?;
    let map_model = |r: &rusqlite::Row| {
        Ok(ProjectModelRow {
            model: r.get(0)?,
            tokens: r.get(1)?,
            input_tokens: r.get(2)?,
            output_tokens: r.get(3)?,
            events: r.get(4)?,
            sessions: r.get(5)?,
            cost_usd: r.get(6)?,
            first_ts: r.get(7)?,
            last_ts: r.get(8)?,
        })
    };
    let by_model: Vec<ProjectModelRow> = if is_unknown {
        stmt_model.query_map(rusqlite::params![cutoff_ts], map_model)?.collect::<Result<Vec<_>, _>>()?
    } else {
        stmt_model.query_map(rusqlite::params![cutoff_ts, project], map_model)?.collect::<Result<Vec<_>, _>>()?
    };

    // by_source
    let sql_src = format!(
        "SELECT u.source, COALESCE(SUM({T}),0), SUM(u.n), COUNT(DISTINCT u.session_id), SUM(u.cost_usd)
         FROM {rows} u
         LEFT JOIN project_alias pj ON pj.alias = u.project
         WHERE {proj_filter} AND {H}
         GROUP BY u.source ORDER BY 2 DESC",
        T = TOKENS,
        H = NOT_HIDDEN
    );
    let mut stmt_src = conn.prepare(&sql_src)?;
    let map_src = |r: &rusqlite::Row| {
        Ok(SourceTotals {
            source: r.get(0)?,
            tokens: r.get(1)?,
            events: r.get(2)?,
            sessions: r.get(3)?,
            cost_usd: r.get(4)?,
        })
    };
    let by_source: Vec<SourceTotals> = if is_unknown {
        stmt_src.query_map(rusqlite::params![cutoff_ts], map_src)?.collect::<Result<Vec<_>, _>>()?
    } else {
        stmt_src.query_map(rusqlite::params![cutoff_ts, project], map_src)?.collect::<Result<Vec<_>, _>>()?
    };

    // daily series
    let sql_daily = format!(
        "SELECT date(ts/1000,'unixepoch') AS d, COALESCE(SUM({T}),0)
         FROM {rows} u
         LEFT JOIN project_alias pj ON pj.alias = u.project
         WHERE {proj_filter} AND {H}
         GROUP BY d ORDER BY d",
        T = TOKENS,
        H = NOT_HIDDEN
    );
    let mut stmt_daily = conn.prepare(&sql_daily)?;
    let map_daily = |r: &rusqlite::Row| {
        Ok(HeatmapCell { date: r.get(0)?, tokens: r.get(1)? })
    };
    let daily: Vec<HeatmapCell> = if is_unknown {
        stmt_daily.query_map(rusqlite::params![cutoff_ts], map_daily)?.collect::<Result<Vec<_>, _>>()?
    } else {
        stmt_daily.query_map(rusqlite::params![cutoff_ts, project], map_daily)?.collect::<Result<Vec<_>, _>>()?
    };

    let dates: Vec<String> = daily.iter().map(|d| d.date.clone()).collect();
    let active_days = dates.len() as i64;
    let (current_streak, longest_streak) = streaks(&dates);
    let peak = daily.iter().max_by_key(|d| d.tokens);

    // sessions_list
    let sql_sessions = format!(
        "SELECT COALESCE(u.session_id, 'unknown'),
                u.source,
                SUM(u.n),
                COALESCE(SUM({T}),0),
                SUM(u.cost_usd),
                MIN(u.min_ts),
                MAX(u.max_ts),
                GROUP_CONCAT(DISTINCT COALESCE(a.canonical, u.model, 'unknown'))
         FROM {rows} u
         LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
         LEFT JOIN project_alias pj ON pj.alias = u.project
         WHERE {proj_filter} AND {H}
         GROUP BY COALESCE(u.session_id, 'unknown'), u.source
         ORDER BY MAX(u.max_ts) DESC",
        T = TOKENS,
        H = NOT_HIDDEN
    );
    let mut stmt_sessions = conn.prepare(&sql_sessions)?;
    let map_session = |r: &rusqlite::Row| {
        let models_str: Option<String> = r.get(7)?;
        let mut models: Vec<String> = models_str
            .map(|s| s.split(',').filter(|m| !m.is_empty()).map(String::from).collect())
            .unwrap_or_default();
        models.sort();
        Ok(SessionRow {
            session_id: r.get(0)?,
            source: r.get(1)?,
            events: r.get(2)?,
            tokens: r.get(3)?,
            cost_usd: r.get(4)?,
            first_ts: r.get(5)?,
            last_ts: r.get(6)?,
            models,
        })
    };
    let sessions_list: Vec<SessionRow> = if is_unknown {
        stmt_sessions.query_map(rusqlite::params![cutoff_ts], map_session)?.collect::<Result<Vec<_>, _>>()?
    } else {
        stmt_sessions.query_map(rusqlite::params![cutoff_ts, project], map_session)?.collect::<Result<Vec<_>, _>>()?
    };

    // total window tokens across all projects
    let total_window_tokens = window_tokens(&conn, cutoff_ts)?;

    Ok(Some(ProjectDetail {
        project: project.to_string(),
        tokens: total,
        input_tokens: inp,
        output_tokens: out,
        reasoning_tokens: reasoning,
        cache_read_tokens: cr,
        cache_write_tokens: cw,
        events,
        sessions,
        cost_usd: cost,
        first_ts,
        last_ts,
        active_days,
        current_streak,
        longest_streak,
        peak_day: peak.map(|p| p.date.clone()),
        peak_day_tokens: peak.map(|p| p.tokens).unwrap_or(0),
        by_model,
        by_source,
        daily,
        sessions_list,
        total_window_tokens,
    }))
}

pub fn heatmap(store: &Store, days: i64) -> DbResult<Vec<HeatmapCell>> {
    let sql = format!(
        "SELECT date(ts/1000,'unixepoch') AS d, COALESCE(SUM({T}),0)
         FROM {R} u WHERE {H} GROUP BY d ORDER BY d",
        T = TOKENS,
        H = NOT_HIDDEN,
        R = usage_rows(Some("?1"))
    );
    let conn = store.read_conn();
    let mut stmt = conn.prepare(&sql)?;
    let rows: Vec<HeatmapCell> = stmt
        .query_map([cutoff(days)], |r| {
            Ok(HeatmapCell { date: r.get(0)?, tokens: r.get(1)? })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn hourly(store: &Store) -> DbResult<Vec<HourRow>> {
    let sql = format!(
        "SELECT CAST(strftime('%H', ts/1000, 'unixepoch') AS INTEGER), COALESCE(SUM({T}),0)
         FROM {R} u WHERE {H} GROUP BY 1 ORDER BY 1",
        T = TOKENS,
        H = NOT_HIDDEN,
        R = usage_rows(None)
    );
    let conn = store.read_conn();
    let mut stmt = conn.prepare(&sql)?;
    let rows: Vec<HourRow> = stmt
        .query_map([], |r| Ok(HourRow { hour: r.get(0)?, tokens: r.get(1)? }))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn peak_days(store: &Store, days: i64) -> DbResult<Vec<PeakDayRow>> {
    let sql = format!(
        "SELECT COALESCE(a.canonical, u.model, 'unknown') AS model,
                date(u.ts/1000, 'unixepoch') AS d,
                COALESCE(SUM({T}), 0) AS tokens,
                COALESCE(SUM(u.cost_usd), 0.0) AS cost
         FROM {R} u
         LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
         WHERE {H}
         GROUP BY 1, 2
         HAVING SUM({T}) > 0
         ORDER BY tokens DESC, d DESC
         LIMIT 5",
        T = TOKENS,
        H = NOT_HIDDEN,
        R = usage_rows(Some("?1"))
    );
    let conn = store.read_conn();
    let mut stmt = conn.prepare(&sql)?;
    let rows: Vec<PeakDayRow> = stmt
        .query_map([cutoff(days)], |r| {
            Ok(PeakDayRow {
                model: r.get(0)?,
                date: r.get(1)?,
                tokens: r.get(2)?,
                cost_usd: r.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// The whole Wrapped card in one read: totals, top models, harness mix, daily
/// totals for the waveform, streaks over the window's own dates, peak day,
/// hour-of-day buckets, top project and the estimated share — all through the
/// same alias-resolving, hidden-model-excluding lens as every other aggregate.
pub fn wrapped_summary(store: &Store, days: i64) -> DbResult<WrappedSummary> {
    let cut = cutoff(days);
    let w = NOT_HIDDEN;
    let rows = usage_rows(Some("?1"));
    let conn = store.read_conn();

    // One row of totals, always present (aggregates without GROUP BY never skip).
    let (total_tokens, events, sessions, first_ts, last_ts, estimated_tokens) = conn
        .query_row(
            &format!(
                "SELECT COALESCE(SUM({T}),0), COALESCE(SUM(u.n),0), COUNT(DISTINCT u.session_id),
                        MIN(u.min_ts), MAX(u.max_ts),
                        COALESCE(SUM(CASE WHEN u.estimated = 1 THEN {T} ELSE 0 END),0)
                 FROM {rows} u WHERE {w}",
                T = TOKENS
            ),
            [cut],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                    r.get::<_, Option<i64>>(4)?,
                    r.get::<_, i64>(5)?,
                ))
            },
        )?;

    let top_models = {
        let mut stmt = conn.prepare(&format!(
            "SELECT COALESCE(a.canonical, u.model, 'unknown'), COALESCE(SUM({T}),0), SUM(u.n)
             FROM {rows} u LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
             WHERE {w} GROUP BY 1 ORDER BY 2 DESC LIMIT 6",
            T = TOKENS
        ))?;
        let rows = stmt
            .query_map([cut], |r| {
                Ok(WrappedModelRow { model: r.get(0)?, tokens: r.get(1)?, events: r.get(2)? })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };

    let by_source = {
        let mut stmt = conn.prepare(&format!(
            "SELECT u.source, COALESCE(SUM({T}),0)
             FROM {rows} u WHERE {w} GROUP BY 1 ORDER BY 2 DESC",
            T = TOKENS
        ))?;
        let rows = stmt
            .query_map([cut], |r| {
                Ok(WrappedSourceRow { source: r.get(0)?, tokens: r.get(1)? })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };

    // Daily totals double as the waveform and the streak/active-day input.
    let daily: Vec<HeatmapCell> = {
        let mut stmt = conn.prepare(&format!(
            "SELECT date(u.ts/1000,'unixepoch') AS d, COALESCE(SUM({T}),0)
             FROM {rows} u WHERE {w} GROUP BY d ORDER BY d",
            T = TOKENS
        ))?;
        let rows = stmt
            .query_map([cut], |r| Ok(HeatmapCell { date: r.get(0)?, tokens: r.get(1)? }))?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    let (current_streak, longest_streak) = streaks(&daily.iter().map(|c| c.date.clone()).collect::<Vec<_>>());
    let active_days = daily.len() as i64;

    let (peak_day, peak_day_tokens) = daily
        .iter()
        .max_by_key(|c| c.tokens)
        .map(|c| (Some(c.date.clone()), c.tokens))
        .unwrap_or((None, 0));

    // Hour-of-day buckets feed both the "power hour" and the night share.
    let hours: Vec<HourRow> = {
        let mut stmt = conn.prepare(&format!(
            "SELECT CAST(strftime('%H', u.ts/1000, 'unixepoch') AS INTEGER), COALESCE(SUM({T}),0)
             FROM {rows} u WHERE {w} GROUP BY 1 ORDER BY 1",
            T = TOKENS
        ))?;
        let rows = stmt
            .query_map([cut], |r| Ok(HourRow { hour: r.get(0)?, tokens: r.get(1)? }))?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    let busiest_hour = hours.iter().max_by_key(|h| h.tokens).map(|h| h.hour);
    let night_tokens: i64 = hours
        .iter()
        .filter(|h| h.hour >= 22 || h.hour < 6)
        .map(|h| h.tokens)
        .sum();
    let night_share = if total_tokens > 0 {
        night_tokens as f64 / total_tokens as f64
    } else {
        0.0
    };

    let (top_project, top_project_tokens) = {
        let mut stmt = conn.prepare(&format!(
            "SELECT {PROJECT}, COALESCE(SUM({T}),0)
             FROM {rows} u LEFT JOIN project_alias pj ON pj.alias = u.project
             WHERE {w} AND {PROJECT} != 'unknown' GROUP BY 1 ORDER BY 2 DESC LIMIT 1",
            T = TOKENS
        ))?;
        let mut rows = stmt
            .query_map([cut], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        rows.pop().map(|(p, t)| (if p == "unknown" { None } else { Some(p) }, t)).unwrap_or((None, 0))
    };

    // For week/month the card compares against the requested window; all-time
    // compares against first-event-to-today, so "X of Y days" reads honestly.
    let window_days = if days > 0 {
        days
    } else {
        first_ts.map(|t| (now_ms() - t) / 86_400_000 + 1).unwrap_or(0)
    };

    Ok(WrappedSummary {
        period_days: days,
        total_tokens,
        events,
        sessions,
        active_days,
        window_days,
        current_streak,
        longest_streak,
        first_ts,
        last_ts,
        top_models,
        by_source,
        daily,
        peak_day,
        peak_day_tokens,
        busiest_hour,
        night_share,
        top_project,
        top_project_tokens,
        estimated_tokens,
    })
}

/// How much of a source's history is the reporting app's own guess.
///
/// Only WackChatter can be anything but zero — every coding harness writes the provider's
/// real usage. Surfaced so the Sources list can say so, rather than letting an estimate
/// pass silently as a measurement.
#[derive(Debug, Serialize)]
pub struct EstimatedShare {
    pub source: String,
    pub events: i64,
    pub estimated: i64,
}

pub fn estimated_share(store: &Store) -> DbResult<Vec<EstimatedShare>> {
    let sql = format!(
        "SELECT source, SUM(u.n), COALESCE(SUM(estimated * u.n),0)
         FROM {R} u WHERE {H} GROUP BY source HAVING SUM(estimated) > 0",
        H = NOT_HIDDEN,
        R = usage_rows(None)
    );
    let conn = store.read_conn();
    let mut stmt = conn.prepare(&sql)?;
    let rows: Vec<EstimatedShare> = stmt
        .query_map([], |r| {
            Ok(EstimatedShare { source: r.get(0)?, events: r.get(1)?, estimated: r.get(2)? })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Date -> that day's (model id, tokens) rows.
type DailyStandings = BTreeMap<String, Vec<(usize, i64)>>;

/// Every visible model's tokens per UTC day; see `daily_standings`.
struct Standings {
    /// Display names, sorted; a model's id is its index here.
    names: Vec<String>,
    days: DailyStandings,
}

impl Standings {
    /// All-time tokens per model id — the same sums as a `GROUP BY` display
    /// name over every visible event, since each day is already one.
    fn totals(&self) -> Vec<i64> {
        let mut totals = vec![0i64; self.names.len()];
        for day in self.days.values() {
            for (id, tokens) in day {
                totals[*id] += tokens;
            }
        }
        totals
    }
}

/// Everything `daily_standings` depends on. The ingest generation catches
/// upserts that change counts in place; row count and newest id catch writes
/// that bypass ingest; the alias segments and hidden names decide which display
/// name a token counts under and whether it counts at all. `now` plays no part:
/// the standings are all-time.
fn standings_key(store: &Store, conn: &rusqlite::Connection) -> DbResult<String> {
    // Two subqueries, not one SELECT: alone, each takes SQLite's O(1)-ish fast
    // path; together they would walk a whole index on every call.
    let (rows, newest): (i64, Option<i64>) = conn.query_row(
        "SELECT (SELECT COUNT(*) FROM usage_event), (SELECT MAX(id) FROM usage_event)",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let aliases: String = conn.query_row(
        "SELECT COALESCE(group_concat(alias || char(31) || canonical || char(31) || from_ts
                                      || char(31) || until_ts, char(30)), '')
         FROM (SELECT * FROM model_alias ORDER BY alias, from_ts)",
        [],
        |r| r.get(0),
    )?;
    let hidden: String = conn.query_row(
        "SELECT COALESCE(group_concat(name, char(30)), '') FROM (SELECT name FROM hidden_model ORDER BY name)",
        [],
        |r| r.get(0),
    )?;
    Ok(format!(
        "{}|{rows}|{}|{aliases}\u{1d}{hidden}",
        store.generation(),
        newest.unwrap_or(0)
    ))
}

/// Every day's per-model token totals, the input both standings replays
/// (leaderboard events and Big-6 tenure) walk through in date order, and the
/// source of all-time model totals and the visible model list.
///
/// Models get dense ids in lexicographic order: id order is exactly the
/// standings' (tokens desc, name asc) tie-break, and every hot structure in a
/// replay is a Vec indexed by id rather than a HashMap keyed by name. Same-day
/// rows are sorted by id, so same-day events come out deterministically.
///
/// A full pass over every event, so it is memoised on the store and recomputed
/// only when `standings_key` changes — a sync that stores something, or a
/// merge, rename or hide.
fn daily_standings(store: &Store) -> DbResult<Arc<Standings>> {
    let key = standings_key(store, &store.read_conn())?;
    store.standings.get_or_compute(key, || compute_daily_standings(store))
}

fn compute_daily_standings(store: &Store) -> DbResult<Standings> {
    let sql = format!(
        "SELECT date(ts/1000,'unixepoch') AS d,
                COALESCE(a.canonical, u.model, 'unknown') AS model,
                COALESCE(SUM({T}),0) AS tokens
         FROM {R} u
         LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
         WHERE {H}
         GROUP BY 1, 2
         ORDER BY d ASC",
        T = TOKENS,
        H = NOT_HIDDEN,
        R = usage_rows(None)
    );
    let conn = store.read_conn();
    let mut stmt = conn.prepare(&sql)?;
    let rows: Vec<(String, String, i64)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<Vec<_>, _>>()?;

    let mut names: Vec<String> = Vec::new();
    {
        let mut seen: HashSet<&str> = HashSet::new();
        for (_, model, _) in &rows {
            if seen.insert(model.as_str()) {
                names.push(model.clone());
            }
        }
    }
    names.sort();
    let id_of: HashMap<&str, usize> = names.iter().enumerate().map(|(i, n)| (n.as_str(), i)).collect();

    let mut days_map: DailyStandings = BTreeMap::new();
    for (d, model, tokens) in rows {
        let id = id_of[model.as_str()];
        days_map.entry(d).or_default().push((id, tokens));
    }
    for day_models in days_map.values_mut() {
        day_models.sort_by_key(|(id, _)| *id);
    }
    Ok(Standings { names, days: days_map })
}

pub fn leaderboard_events(store: &Store, days: i64) -> DbResult<Vec<LeaderboardEvent>> {
    let cutoff_ts = cutoff(days);
    let cutoff_date: String = store.read_conn().query_row(
        "SELECT date(?1/1000, 'unixepoch')",
        [cutoff_ts],
        |r| r.get(0),
    )?;
    let is_all_time = days >= 3650 || days <= 0;

    let standings = daily_standings(store)?;
    let (names, days_map) = (&standings.names, &standings.days);
    let n_models = names.len();
    let conn = store.read_conn();

    let mut aliased_models: HashSet<String> = HashSet::new();
    if let Ok(mut alias_stmt) = conn.prepare("SELECT alias FROM model_alias UNION SELECT canonical FROM model_alias") {
        if let Ok(alias_rows) = alias_stmt.query_map([], |r| r.get::<_, String>(0)) {
            for name in alias_rows.flatten() {
                aliased_models.insert(name);
            }
        }
    }

    let mut cumulative: Vec<i64> = vec![0; n_models];
    // Standings, kept ordered by (tokens desc, id asc). Only the day's active
    // models move each day, so maintaining this is O(active models) per day —
    // no clone-and-resort of the whole model list like the per-day rebuild.
    // A model enters the standings when its first row lands (the remove below
    // is a no-op until then): the old rebuild only ever contained seen models,
    // and the Big-6 guard `prev_top6.len() == 6` depends on that — phantom
    // zero-token entries would promote and demote against models that don't
    // exist yet.
    let mut order: BTreeSet<(Reverse<i64>, usize)> = BTreeSet::new();
    let mut first_seen: Vec<Option<String>> = vec![None; n_models];
    let mut ever_top5: HashSet<usize> = HashSet::new();
    let mut pre_window_max: Vec<Option<i64>> = vec![None; n_models];
    let mut in_window_max: Vec<Option<i64>> = vec![None; n_models];
    // Date each top-6 member last ENTERED the Big 6 — not the date it took its
    // current rank, so a long-reigning #1 that slides down through the ranks
    // still counts its whole run when it finally exits.
    let mut top6_since: Vec<Option<String>> = vec![None; n_models];
    // Yesterday's ranks, materialized once at the window boundary: the
    // standings after the last pre-window day, before the first window day's
    // tokens land. Stays None until then, which is what keeps day-1 overtakes
    // (there is nothing to have been overtaken from) off.
    let mut prev_rank: Option<Vec<usize>> = None;
    let mut prev_top6: Vec<usize> = Vec::new();
    let mut seen_pre_window = false;

    let mut events: Vec<LeaderboardEvent> = Vec::new();

    for (d, day_models) in days_map {
        let in_window = d >= &cutoff_date;
        if !in_window {
            seen_pre_window = true;
        }

        // 1. First seen & record days
        for (model, daily_tokens) in day_models {
            if first_seen[*model].is_none() {
                first_seen[*model] = Some(d.clone());
                if in_window && !aliased_models.contains(&names[*model]) {
                    events.push(LeaderboardEvent {
                        kind: "first_seen".into(),
                        model: names[*model].clone(),
                        other_model: None,
                        rank: None,
                        date: d.clone(),
                        tokens: *daily_tokens,
                        tenure_days: None,
                    });
                }
            }

            if !in_window {
                let cur = pre_window_max[*model].get_or_insert(*daily_tokens);
                if *daily_tokens > *cur {
                    *cur = *daily_tokens;
                }
            } else if let Some(pre_max) = pre_window_max[*model] {
                let cur_record = in_window_max[*model].unwrap_or(pre_max);
                if *daily_tokens > cur_record {
                    in_window_max[*model] = Some(*daily_tokens);
                    events.push(LeaderboardEvent {
                        kind: "record".into(),
                        model: names[*model].clone(),
                        other_model: None,
                        rank: None,
                        date: d.clone(),
                        tokens: *daily_tokens,
                        tenure_days: None,
                    });
                }
            }
        }

        // 2. Snapshot the standings once, on the first window day — exactly the
        //    state this window's overtakes and Big-6 movements compare against.
        if in_window && prev_rank.is_none() && seen_pre_window {
            let mut ranks = vec![0usize; n_models];
            for (idx, &(_, id)) in order.iter().enumerate() {
                ranks[id] = idx + 1;
            }
            prev_rank = Some(ranks);
        }

        // 3. Cumulative totals; the ordered set above stays current.
        for (model, daily_tokens) in day_models {
            let old = cumulative[*model];
            order.remove(&(Reverse(old), *model));
            let new = old + daily_tokens;
            cumulative[*model] = new;
            order.insert((Reverse(new), *model));
        }

        let curr_top6: Vec<usize> = order.iter().take(6).map(|&(_, id)| id).collect();

        // 4. Ranks for today. Only window days emit rank-bearing events, so the
        //    full rank map is built only on those days. curr_order is the same
        //    ordering as a rank-indexed vec, for the overtake scan below.
        let mut curr_rank: Vec<usize> = Vec::new();
        let mut curr_order: Vec<usize> = Vec::new();
        if in_window {
            curr_rank.resize(n_models, 0);
            for (idx, &(_, id)) in order.iter().enumerate() {
                curr_rank[id] = idx + 1;
                curr_order.push(id);
            }
        }

        // 5. Debut (entered top 5 for the first time with prior history,
        //    suppressed on ALL filter)
        if in_window && !is_all_time {
            for (model, _) in day_models {
                let rank = curr_rank[*model];
                if rank <= 5 && !ever_top5.contains(model) {
                    let has_prior_history = first_seen[*model]
                        .as_ref()
                        .map(|fd| fd < d)
                        .unwrap_or(false);
                    if has_prior_history {
                        events.push(LeaderboardEvent {
                            kind: "debut".into(),
                            model: names[*model].clone(),
                            other_model: None,
                            rank: Some(rank as i64),
                            date: d.clone(),
                            tokens: cumulative[*model],
                            tenure_days: None,
                        });
                    }
                }
            }
        }

        // Mark models in top 5 so far
        for &(_, id) in order.iter().take(5) {
            ever_top5.insert(id);
        }

        // 6. Big 6 movements (promotion / demotion on ALL filter)
        let mut promoted_models: Vec<usize> = Vec::new();
        let mut demoted_models: Vec<usize> = Vec::new();

        if in_window && prev_top6.len() == 6 && curr_top6.len() == 6 {
            for m in &curr_top6 {
                if !prev_top6.contains(m) {
                    promoted_models.push(*m);
                }
            }
            for m in &prev_top6 {
                if !curr_top6.contains(m) {
                    demoted_models.push(*m);
                }
            }
        }

        if in_window && is_all_time && !promoted_models.is_empty() {
            let mut sorted_promoted = promoted_models.clone();
            sorted_promoted.sort_by_key(|m| curr_rank[*m]);
            let mut sorted_demoted = demoted_models.clone();
            sorted_demoted.sort_by_key(|m| prev_top6.iter().position(|x| x == m).unwrap_or(99));

            for (i, promoted) in sorted_promoted.iter().enumerate() {
                let demoted = sorted_demoted.get(i);
                let promo_rank = curr_rank[*promoted];

                events.push(LeaderboardEvent {
                    kind: "promotion".into(),
                    model: names[*promoted].clone(),
                    other_model: demoted.map(|d| names[*d].clone()),
                    rank: Some(promo_rank as i64),
                    date: d.clone(),
                    tokens: cumulative[*promoted],
                    tenure_days: None,
                });

                if let Some(demoted_name) = demoted {
                    let demoted_rank =
                        prev_top6.iter().position(|x| x == demoted_name).unwrap_or(5) + 1;
                    let tenure = days_between(
                        top6_since[*demoted_name].as_deref().unwrap_or(d),
                        d,
                    )
                    .unwrap_or(0);

                    events.push(LeaderboardEvent {
                        kind: "demotion".into(),
                        model: names[*demoted_name].clone(),
                        other_model: Some(names[*promoted].clone()),
                        rank: Some(demoted_rank as i64),
                        date: d.clone(),
                        tokens: cumulative[*demoted_name],
                        tenure_days: Some(tenure),
                    });
                }
            }

            if sorted_demoted.len() > sorted_promoted.len() {
                for demoted_name in &sorted_demoted[sorted_promoted.len()..] {
                    let demoted_rank =
                        prev_top6.iter().position(|x| x == demoted_name).unwrap_or(5) + 1;
                    let tenure = days_between(
                        top6_since[*demoted_name].as_deref().unwrap_or(d),
                        d,
                    )
                    .unwrap_or(0);
                    events.push(LeaderboardEvent {
                        kind: "demotion".into(),
                        model: names[*demoted_name].clone(),
                        other_model: None,
                        rank: Some(demoted_rank as i64),
                        date: d.clone(),
                        tokens: cumulative[*demoted_name],
                        tenure_days: Some(tenure),
                    });
                }
            }
        }

        // 7. Overtakes (adjacent rank swaps)
        if let Some(prev_ranks) = &prev_rank {
            if in_window {
                // Models sorted by yesterday's rank. A model X overtakes exactly
                // the models that sat above it yesterday and sit below it today,
                // so scanning the shorter of those two sides finds them all —
                // no all-models sweep per active model. Rank numbers alone
                // cannot bound the search: a third model rising past X while X
                // rises past Y leaves both rank numbers unchanged, and a model
                // first seen today was in nobody's yesterday.
                let mut prev_order: Vec<usize> = vec![0; n_models];
                let mut prev_seen = 0usize;
                for m in 0..n_models {
                    if prev_ranks[m] > 0 {
                        prev_order[prev_ranks[m] - 1] = m;
                        prev_seen += 1;
                    }
                }
                prev_order.truncate(prev_seen);

                for (model, _) in day_models {
                    let px = prev_ranks[*model];
                    let cx = curr_rank[*model];
                    let above_yesterday = px.saturating_sub(1);
                    let below_today = curr_order.len() - cx.min(curr_order.len());

                    let mut hits: Vec<usize> = Vec::new();
                    if above_yesterday <= below_today {
                        for &other in &prev_order[..above_yesterday] {
                            if curr_rank[other] > cx {
                                hits.push(other);
                            }
                        }
                    } else {
                        for &other in &curr_order[cx..] {
                            if prev_ranks[other] > 0 && prev_ranks[other] < px {
                                hits.push(other);
                            }
                        }
                    }

                    // Passed models are listed in the rank order they were
                    // passed from, so same-day events come out deterministically.
                    hits.sort_by_key(|&other| prev_ranks[other]);

                    for other in hits {
                        if is_all_time
                            && promoted_models.contains(model)
                            && demoted_models.contains(&other)
                        {
                            continue; // already told as a Big-6 promotion/demotion
                        }

                        let gap = cumulative[*model] - cumulative[other];
                        if gap > 0 {
                            events.push(LeaderboardEvent {
                                kind: "overtake".into(),
                                model: names[*model].clone(),
                                other_model: Some(names[other].clone()),
                                rank: Some(cx as i64),
                                date: d.clone(),
                                tokens: gap,
                                tenure_days: None,
                            });
                        }
                    }
                }
            }
        }

        // 8. Big-6 tenure bookkeeping: an entry date is stamped when a model
        //    (re-)joins the top 6 and cleared whenever it isn't there — a rank
        //    shuffle inside the top 6 never resets it. Clearing is not gated
        //    on demoted_models: that list only exists once a full 6-member
        //    top 6 has formed, and an early-days departure must not carry a
        //    stale entry date into a later re-entry.
        for (id, since) in top6_since.iter_mut().enumerate() {
            if since.is_some() && !curr_top6.contains(&id) {
                *since = None;
            }
        }
        for model in &curr_top6 {
            if top6_since[*model].is_none() {
                top6_since[*model] = Some(d.clone());
            }
        }

        prev_top6 = curr_top6;
        if in_window {
            prev_rank = Some(std::mem::take(&mut curr_rank));
        }
    }

    events.sort_by(|a, b| b.date.cmp(&a.date));
    events.truncate(60);
    Ok(events)
}

/// One unbroken run in the all-time Big 6: from the day a model joined the
/// top 6 to the day it was first seen outside it (`exited = None` while the
/// run is still going, in which case `days` counts up to today).
#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct Big6Stint {
    pub entered: String,
    pub exited: Option<String>,
    pub days: i64,
}

/// A model's whole all-time Big-6 record. The leaderboard feed tells each
/// entry and exit once and then ages it out; this is the part that stays.
#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct Big6History {
    pub total_days: i64,
    pub longest_days: i64,
    /// Oldest first.
    pub stints: Vec<Big6Stint>,
}

/// Replays the all-time standings for one (display-name) model and returns
/// every Big-6 run it had, or None if it never made the top 6.
///
/// Runs are measured the way the leaderboard's demotion tenure is — entry date
/// to exit date, a rank shuffle inside the top 6 never resets it, and the
/// early days before six models exist count — so a finished run here always
/// matches the "Nd in Big 6" its exit event showed.
pub fn model_big6(store: &Store, model: &str) -> DbResult<Option<Big6History>> {
    let standings = daily_standings(store)?;
    let (names, days_map) = (&standings.names, &standings.days);
    let Ok(target) = names.binary_search_by(|n| n.as_str().cmp(model)) else {
        return Ok(None);
    };

    let mut cumulative: Vec<i64> = vec![0; names.len()];
    let mut order: BTreeSet<(Reverse<i64>, usize)> = BTreeSet::new();
    let mut since: Option<String> = None;
    let mut stints: Vec<Big6Stint> = Vec::new();

    for (d, day_models) in days_map {
        for (id, daily_tokens) in day_models {
            let old = cumulative[*id];
            order.remove(&(Reverse(old), *id));
            cumulative[*id] = old + daily_tokens;
            order.insert((Reverse(old + daily_tokens), *id));
        }
        let in_top6 = order.iter().take(6).any(|&(_, id)| id == target);
        match (&since, in_top6) {
            (None, true) => since = Some(d.clone()),
            (Some(entered), false) => {
                stints.push(Big6Stint {
                    entered: entered.clone(),
                    exited: Some(d.clone()),
                    days: days_between(entered, d).unwrap_or(0),
                });
                since = None;
            }
            _ => {}
        }
    }
    if let Some(entered) = since {
        let day = format_description!("[year]-[month]-[day]");
        let today = time::OffsetDateTime::now_utc().date().format(&day).unwrap_or_default();
        let days = days_between(&entered, &today).unwrap_or(0).max(0);
        stints.push(Big6Stint { entered, exited: None, days });
    }

    if stints.is_empty() {
        return Ok(None);
    }
    Ok(Some(Big6History {
        total_days: stints.iter().map(|s| s.days).sum(),
        longest_days: stints.iter().map(|s| s.days).max().unwrap_or(0),
        stints,
    }))
}

/// One unbroken run holding a single Big-6 rank: from the day the model took
/// the seat to the day the next model did (`exited = None` while it still
/// holds it, in which case `days` counts up to today).
#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct Big6SeatRun {
    pub model: String,
    pub entered: String,
    pub exited: Option<String>,
    pub days: i64,
}

/// Every run at one rank — the history of who held that seat. Once a rank is
/// first filled its runs tile contiguously: each run's `exited` is the next
/// run's `entered`.
#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct Big6Seat {
    pub rank: i64,
    pub runs: Vec<Big6SeatRun>,
}

/// Seat occupancy over the whole standings history — the Trends "Podium"
/// panel. `first_date` is the earliest recorded day so the frontend can clamp
/// its zoom window to real history.
#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct Big6Timeline {
    pub first_date: Option<String>,
    pub seats: Vec<Big6Seat>,
}

/// Replays the all-time standings once and records every run at each of the
/// six ranks. Where `model_big6` tracks one model's membership in the top 6,
/// this tracks who held which seat — a slide from #2 to #5 closes a run at #2
/// and opens one at #5 without the model leaving the Big 6, so a model's seat
/// days always sum back to its membership days.
pub fn big6_timeline(store: &Store) -> DbResult<Big6Timeline> {
    let standings = daily_standings(store)?;
    let (names, days_map) = (&standings.names, &standings.days);

    let mut cumulative: Vec<i64> = vec![0; names.len()];
    let mut order: BTreeSet<(Reverse<i64>, usize)> = BTreeSet::new();
    // (model id, day the current run at this seat started)
    let mut holder: [Option<(usize, String)>; 6] = Default::default();
    let mut seats: Vec<Big6Seat> = (1..=6)
        .map(|rank| Big6Seat {
            rank,
            runs: Vec::new(),
        })
        .collect();

    for (d, day_models) in days_map {
        for (id, daily_tokens) in day_models {
            let old = cumulative[*id];
            order.remove(&(Reverse(old), *id));
            cumulative[*id] = old + daily_tokens;
            order.insert((Reverse(old + daily_tokens), *id));
        }
        let top: Vec<usize> = order.iter().take(6).map(|&(_, id)| id).collect();
        for (rank, &m) in top.iter().enumerate() {
            if holder[rank].as_ref().map(|(h, _)| *h) == Some(m) {
                continue;
            }
            if let Some((prev, entered)) = holder[rank].take() {
                seats[rank].runs.push(Big6SeatRun {
                    model: names[prev].clone(),
                    exited: Some(d.clone()),
                    days: days_between(&entered, d).unwrap_or(0),
                    entered,
                });
            }
            holder[rank] = Some((m, d.clone()));
        }
    }

    let day = format_description!("[year]-[month]-[day]");
    let today = time::OffsetDateTime::now_utc()
        .date()
        .format(&day)
        .unwrap_or_default();
    for (rank, h) in holder.iter().enumerate() {
        if let Some((m, entered)) = h {
            seats[rank].runs.push(Big6SeatRun {
                model: names[*m].clone(),
                entered: entered.clone(),
                exited: None,
                days: days_between(entered, &today).unwrap_or(0).max(0),
            });
        }
    }

    Ok(Big6Timeline {
        first_date: days_map.keys().next().cloned(),
        seats,
    })
}

/// (current, longest) consecutive-day streaks over a sorted list of ISO dates.
/// Days are UTC; the current streak tolerates "today hasn't happened yet".
fn streaks(dates: &[String]) -> (i64, i64) {
    let day = format_description!("[year]-[month]-[day]");
    let today = time::OffsetDateTime::now_utc().date().format(&day).unwrap_or_default();
    let set: std::collections::HashSet<&String> = dates.iter().collect();

    let mut longest = 0i64;
    let mut run = 0i64;
    let mut prev: Option<time::Date> = None;
    for d in dates {
        let Ok(parsed) = time::Date::parse(d, &day) else { continue };
        run = match prev {
            Some(p) if next_day(p) == parsed => run + 1,
            _ => 1,
        };
        longest = longest.max(run);
        prev = Some(parsed);
    }

    if dates.is_empty() {
        return (0, longest);
    }
    let mut cursor = today.clone();
    if !set.contains(&cursor) {
        match shift_day(&today, -1) {
            Some(y) => cursor = y,
            None => return (0, longest),
        }
    }
    let mut current = 0i64;
    while set.contains(&cursor) {
        current += 1;
        match shift_day(&cursor, -1) {
            Some(y) => cursor = y,
            None => break,
        }
    }

    (current, longest)
}

fn next_day(d: time::Date) -> time::Date {
    d.next_day().unwrap_or(d)
}

fn shift_day(s: &str, delta: i64) -> Option<String> {
    let day = format_description!("[year]-[month]-[day]");
    let mut d = time::Date::parse(s, &day).ok()?;
    let steps = delta.unsigned_abs();
    for _ in 0..steps {
        d = if delta < 0 { d.previous_day()? } else { d.next_day()? };
    }
    d.format(&day).ok()
}

fn days_between(from_str: &str, to_str: &str) -> Option<i64> {
    let day = format_description!("[year]-[month]-[day]");
    let d1 = time::Date::parse(from_str, &day).ok()?;
    let d2 = time::Date::parse(to_str, &day).ok()?;
    Some((d2 - d1).whole_days())
}

/// RFC3339 -> epoch ms, shared by the collectors.
pub fn parse_ts_ms(s: &str) -> Option<i64> {
    time::OffsetDateTime::parse(s, &Rfc3339)
        .ok()
        .map(|d| d.unix_timestamp_nanos() as i64 / 1_000_000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Source, UsageEvent};

    #[test]
    fn streak_math() {
        let today = time::OffsetDateTime::now_utc().date().format(&format_description!("[year]-[month]-[day]")).unwrap();
        let yesterday = shift_day(&today, -1).unwrap();
        let two_ago = shift_day(&yesterday, -1).unwrap();
        // today + yesterday + day-before: current 3, longest 3
        let (cur, long) = streaks(&[two_ago.clone(), yesterday.clone(), today.clone()]);
        assert_eq!((cur, long), (3, 3));
        // only yesterday and the day before: current streak should be 2 (today not started yet)
        let (cur, _) = streaks(&[two_ago.clone(), yesterday.clone()]);
        assert_eq!(cur, 2);
        // gap breaks the run
        let older = shift_day(&two_ago, -5).unwrap();
        let (_, long) = streaks(&[older, two_ago, yesterday, today]);
        assert_eq!(long, 3);
        assert_eq!(streaks(&[]), (0, 0));
    }

    fn test_event(model: &str, ts: i64, input: i64) -> UsageEvent {
        UsageEvent {
            source: Source::Zcode,
            source_event_id: format!("{model}-{ts}"),
            ts,
            session_id: None,
            project: None,
            provider: None,
            provider_name: None,
            model: Some(model.to_string()),
            input_tokens: input,
            output_tokens: 0,
            reasoning_tokens: None,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            duration_ms: None,
            ttft_ms: None,
            is_subagent: false,
            estimated: false,
            purpose: None, outcome: None, workspace: None, subagent_id: None, mode: None,
        }
    }

    #[test]
    fn dated_alias_splits_floating_model_at_cutover() {
        // deepseek-flash always names the newest Flash: renamed to V4.1, then
        // split when V4.2 ships, history before the cutover stays V4.1.
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        let cut = now - 10_000;
        store
            .insert_events(&[
                test_event("deepseek-flash", cut - 5_000, 100),
                test_event("deepseek-flash", cut + 5_000, 40),
            ])
            .unwrap();
        store.rename_model("deepseek-flash", "DeepSeek V4.1 Flash").unwrap();
        store.split_model_alias("deepseek-flash", cut, "DeepSeek V4.2 Flash").unwrap();

        let rows = by_model(&store, 3650).unwrap();
        let tok = |m: &str| rows.iter().find(|r| r.model == m).map(|r| r.tokens);
        assert_eq!(rows.len(), 2);
        assert_eq!(tok("DeepSeek V4.1 Flash"), Some(100));
        assert_eq!(tok("DeepSeek V4.2 Flash"), Some(40));
        let detail = model_detail(&store, "DeepSeek V4.1 Flash", 3650).unwrap().unwrap();
        assert_eq!(detail.tokens, 100);

        // Hiding the old name hides only the old range.
        store.hide_models(&["DeepSeek V4.1 Flash".into()]).unwrap();
        let rows = by_model(&store, 3650).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].model, "DeepSeek V4.2 Flash");
        assert_eq!(rows[0].tokens, 40);
    }

    /// `daily_standings` is memoised. Anything that changes which tokens count, or
    /// under which name, must change its key — or the Big-6, leaderboard, podium,
    /// champion badges and family lists would keep showing the old numbers.
    #[test]
    fn memoised_standings_follow_every_change() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        let totals = |store: &Store| -> Vec<(String, i64)> {
            let s = daily_standings(store).unwrap();
            s.names.iter().cloned().zip(s.totals()).collect()
        };
        store
            .insert_events(&[test_event("alpha", now - 2000, 100), test_event("beta", now - 1000, 50)])
            .unwrap();
        assert_eq!(totals(&store), vec![("alpha".into(), 100), ("beta".into(), 50)]);
        assert!(
            Arc::ptr_eq(&daily_standings(&store).unwrap(), &daily_standings(&store).unwrap()),
            "unchanged inputs reuse the memo"
        );

        // A late-finalized count rewrites a row in place: same row count, same ids.
        store.insert_events(&[test_event("beta", now - 1000, 500)]).unwrap();
        assert_eq!(totals(&store), vec![("alpha".into(), 100), ("beta".into(), 500)]);

        store.hide_models(&["alpha".into()]).unwrap();
        assert_eq!(totals(&store), vec![("beta".into(), 500)]);
        store.unhide_model("alpha").unwrap();
        assert_eq!(totals(&store), vec![("alpha".into(), 100), ("beta".into(), 500)]);

        store.merge_models(&["alpha".into(), "beta".into()], "alpha").unwrap();
        assert_eq!(totals(&store), vec![("alpha".into(), 600)]);

        // A write that bypasses ingest still shows up (row count and newest id).
        store
            .read_conn()
            .execute(
                "INSERT INTO usage_event (source, source_event_id, ts, model, input_tokens)
                 VALUES ('zcode', 'raw-1', ?1, 'gamma', 7)",
                [now],
            )
            .unwrap();
        assert_eq!(totals(&store), vec![("alpha".into(), 600), ("gamma".into(), 7)]);
    }

    #[test]
    fn every_alias_join_is_time_ranged() {
        // A bare `ON a.alias = u.model` would apply one segment's name to all
        // time and, with several segments, count an event once per segment.
        let src = include_str!("aggregate.rs");
        let want = crate::store::MODEL_ALIAS_JOIN;
        // Split so this test's own source does not match itself.
        for (i, _) in src.match_indices(concat!("LEFT JOIN ", "model_alias a ON")) {
            assert!(src[i..].starts_with(want), "un-ranged alias join at byte {i}");
        }
    }

    #[test]
    fn model_aliases_fold_variants() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        store
            .insert_events(&[test_event("GLM-5.3", now, 100), test_event("glm-5.3", now - 1000, 200)])
            .unwrap();

        // Before merging, the same model appears twice.
        let stats = model_stats(&store, 3650).unwrap();
        assert_eq!(stats.len(), 2);

        store
            .merge_models(&["GLM-5.3".into(), "glm-5.3".into()], "GLM-5.3")
            .unwrap();

        // model_stats folds both variants under the canonical name, summing tokens.
        let stats = model_stats(&store, 3650).unwrap();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].model, "GLM-5.3");
        assert_eq!(stats[0].tokens, 300);
        assert_eq!(stats[0].events, 2);

        // by_model and daily_by_model resolve the same way.
        let by_model = by_model(&store, 3650).unwrap();
        assert_eq!(by_model.len(), 1);
        assert_eq!(by_model[0].model, "GLM-5.3");
        assert_eq!(by_model[0].tokens, 300);

        let daily = daily_by_model(&store, 3650).unwrap();
        assert_eq!(daily.len(), 1);
        assert_eq!(daily[0].model, "GLM-5.3");
        assert_eq!(daily[0].tokens, 300);

        // Unmerging restores the original two rows.
        store.remove_aliases_for("GLM-5.3").unwrap();
        let stats = model_stats(&store, 3650).unwrap();
        assert_eq!(stats.len(), 2);
    }

    /// `avg_monthly_tokens` divides total tokens by the calendar months that
    /// have any usage; idle gap months never enter the divisor.
    #[test]
    fn overview_monthly_average_counts_months_with_usage() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();

        // Empty store: nothing to average.
        let ov = overview(&store).unwrap();
        assert_eq!(ov.months_tracked, 0);
        assert_eq!(ov.avg_monthly_tokens, 0.0);

        // Two events in January 2024, one in February — 600 tokens over 2 months.
        store
            .insert_events(&[
                test_event("glm-5.3", 1705320000000, 100), // 2024-01-15
                test_event("glm-5.3", 1705752000000, 200), // 2024-01-20
                test_event("glm-5.3", 1707134400000, 300), // 2024-02-05
            ])
            .unwrap();
        let ov = overview(&store).unwrap();
        assert_eq!(ov.total_tokens, 600);
        assert_eq!(ov.months_tracked, 2);
        assert_eq!(ov.avg_monthly_tokens, 300.0);

        // A March gap then April usage: April counts, March does not — 1000/3.
        store
            .insert_events(&[test_event("glm-5.3", 1712318400000, 400)]) // 2024-04-05
            .unwrap();
        let ov = overview(&store).unwrap();
        assert_eq!(ov.months_tracked, 3);
        assert!((ov.avg_monthly_tokens - 1000.0 / 3.0).abs() < 1e-9);

        // Hidden models leave both the total and the month count (NOT_HIDDEN).
        store.hide_models(&["glm-5.3".into()]).unwrap();
        let ov = overview(&store).unwrap();
        assert_eq!(ov.months_tracked, 0);
        assert_eq!(ov.avg_monthly_tokens, 0.0);
    }

    /// UTC midnight-relative timestamp `days_ago` back at hour `h`, so hour
    /// buckets are deterministic no matter when the test runs.
    fn ts_at(days_ago: i64, h: u8) -> i64 {
        let d = time::OffsetDateTime::now_utc().date() - time::Duration::days(days_ago);
        d.with_time(time::Time::from_hms(h, 30, 0).unwrap())
            .assume_utc()
            .unix_timestamp()
            * 1000
    }

    fn wrapped_event(
        id: &str,
        model: &str,
        ts: i64,
        input: i64,
        session: Option<&str>,
        project: Option<&str>,
        estimated: bool,
    ) -> UsageEvent {
        UsageEvent {
            source_event_id: id.to_string(),
            session_id: session.map(str::to_string),
            project: project.map(str::to_string),
            estimated,
            ..test_event(model, ts, input)
        }
    }

    #[test]
    fn wrapped_summary_windows_totals_and_persona_inputs() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        store
            .insert_events(&[
                // yesterday 23:30 — the night owl event, and the only estimated one
                wrapped_event("w1", "claude-sonnet-5", ts_at(1, 23), 100, Some("s1"), Some("/repo/x"), true),
                wrapped_event("w2", "claude-sonnet-5", ts_at(1, 10), 50, Some("s2"), Some("/repo/y"), false),
                wrapped_event("w3", "gpt-5.6", ts_at(1, 10), 30, Some("s1"), None, false),
                // ten days back: outside every window below except all-time
                wrapped_event("w4", "gpt-5.6", ts_at(10, 12), 999, Some("old"), None, false),
            ])
            .unwrap();

        let s = wrapped_summary(&store, 2).unwrap();
        assert_eq!((s.period_days, s.window_days), (2, 2));
        assert_eq!(s.total_tokens, 180);
        assert_eq!(s.events, 3);
        assert_eq!(s.sessions, 2);
        assert_eq!(s.active_days, 1);
        assert_eq!((s.current_streak, s.longest_streak), (1, 1));
        assert_eq!(s.top_models.len(), 2);
        assert_eq!(s.top_models[0].model, "claude-sonnet-5");
        assert_eq!((s.top_models[0].tokens, s.top_models[0].events), (150, 2));
        assert_eq!(s.by_source.len(), 1);
        assert_eq!(s.by_source[0].source, "zcode");
        assert_eq!(s.by_source[0].tokens, 180);
        let yesterday = shift_day(
            &time::OffsetDateTime::now_utc()
                .date()
                .format(&format_description!("[year]-[month]-[day]"))
                .unwrap(),
            -1,
        )
        .unwrap();
        assert_eq!(s.peak_day.as_deref(), Some(yesterday.as_str()));
        assert_eq!(s.peak_day_tokens, 180);
        assert_eq!(s.busiest_hour, Some(23));
        assert!((s.night_share - 100.0 / 180.0).abs() < 1e-9);
        assert_eq!(s.top_project.as_deref(), Some("/repo/x"));
        assert_eq!(s.top_project_tokens, 100);
        assert_eq!(s.estimated_tokens, 100);

        // All-time folds the old event back in; the window spans to first use.
        let all = wrapped_summary(&store, 0).unwrap();
        assert_eq!(all.total_tokens, 1179);
        assert_eq!(all.events, 4);
        assert_eq!(all.top_models[0].model, "gpt-5.6");
        assert_eq!(all.top_models[0].tokens, 1029);
        assert!(all.window_days >= 10);
    }

    fn project_event(id: &str, project: &str, ts: i64, input: i64) -> UsageEvent {
        UsageEvent {
            project: Some(project.to_string()),
            session_id: Some(format!("s-{id}")),
            source_event_id: id.to_string(),
            ..test_event("gpt-5", ts, input)
        }
    }

    fn wackcode_event(id: &str, ts: i64, input: i64) -> UsageEvent {
        UsageEvent {
            source: Source::WackCode,
            source_event_id: id.to_string(),
            ts,
            session_id: Some("chat-1".to_string()),
            project: Some("/repo".to_string()),
            provider: Some("anthropic".to_string()),
            provider_name: None,
            model: Some("claude-sonnet-4-5".to_string()),
            input_tokens: input,
            output_tokens: 0,
            reasoning_tokens: None,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            duration_ms: Some(1000),
            ttft_ms: None,
            is_subagent: false,
            estimated: false,
            purpose: Some("chat".to_string()),
            outcome: Some("completed".to_string()),
            workspace: None,
            subagent_id: None,
            mode: None,
        }
    }

    #[test]
    fn wackcode_detail_breaks_down_purpose_outcome_and_sessions() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        // mid-day 2025-10-09 UTC, so the four calls provably share one date
        let base = 1_760_000_000_000_i64;
        let mut child = wackcode_event("w2", base + 1000, 50);
        child.purpose = Some("subagent".into());
        child.subagent_id = Some("sub-1".into());
        child.is_subagent = true;
        child.duration_ms = Some(4000);
        let mut title = wackcode_event("w3", base + 2000, 5);
        title.purpose = Some("title".into());
        title.model = Some("gpt-5-mini".into());
        title.duration_ms = Some(200);
        let mut failed = wackcode_event("w4", base + 3000, 0);
        failed.outcome = Some("failed".into());
        failed.duration_ms = Some(300);
        store
            .insert_events(&[
                wackcode_event("w1", base, 100),
                child,
                title,
                failed,
            ])
            .unwrap();

        let d = wackcode_detail(&store, 3650).unwrap();
        assert_eq!(d.events, 4);
        assert_eq!(d.sessions, 1);
        assert_eq!(d.active_days, 1);
        assert_eq!(d.total_tokens, 155);
        assert_eq!(d.subagent_events, 1);
        assert_eq!(d.subagent_tokens, 50);
        let purposes: Vec<&str> = d.by_purpose.iter().map(|p| p.purpose.as_str()).collect();
        // pre-mode records: the main thread surfaces as 'code'
        assert_eq!(purposes, vec!["code", "subagent", "title"]);
        assert_eq!(d.by_mode.len(), 1);
        assert_eq!(d.by_mode[0].name, "code");
        assert_eq!(
            d.by_outcome
                .iter()
                .find(|o| o.outcome == "failed")
                .unwrap()
                .events,
            1
        );
        // two distinct models ran inside the one chat
        let models: Vec<&str> = d.by_model.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(models, vec!["claude-sonnet-4-5", "gpt-5-mini"]);
        assert_eq!(d.by_provider.len(), 1);
        assert_eq!(d.by_project.len(), 1);

        let s = &d.sessions_list[0];
        assert_eq!(s.session_id, "chat-1");
        assert_eq!(s.project, "/repo");
        // housekeeping calls stay out of the headline models: the title call's
        // gpt-5-mini shows in the expanded timeline only
        assert_eq!(s.models, vec!["claude-sonnet-4-5"]);
        assert_eq!(s.mode, "code");
        assert_eq!(s.events, 4);
        assert_eq!(s.subagents, 1);
        assert_eq!(s.completed, 3);
        assert_eq!(s.failed, 1);
        assert_eq!(s.cancelled, 0);

        let calls = wackcode_session_calls(&store, "chat-1").unwrap();
        assert_eq!(calls.len(), 4);
        assert_eq!(calls.iter().map(|c| c.purpose.as_str()).collect::<Vec<_>>(), vec!["code", "subagent", "title", "code"]);

        // the window respects the days cutoff
        let d7 = wackcode_detail(&store, 7).unwrap();
        assert_eq!(d7.events, 0);
    }

    #[test]
    fn wackcode_chat_mode_splits_from_code() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let base = 1_760_000_000_000_i64;
        // a chat-mode session: main call and its title call carry mode 'chat'
        let mut main = wackcode_event("m1", base, 100);
        main.session_id = Some("chat-9".to_string());
        main.mode = Some("chat".into());
        let mut title = wackcode_event("m2", base + 1000, 5);
        title.session_id = Some("chat-9".to_string());
        title.purpose = Some("title".into());
        title.model = Some("gpt-5-mini".into());
        title.mode = Some("chat".into());
        // a code-mode session in the same window
        store.insert_events(&[wackcode_event("m3", base + 500, 40), main, title]).unwrap();

        let d = wackcode_detail(&store, 3650).unwrap();
        let purposes: Vec<(&str, i64)> =
            d.by_purpose.iter().map(|p| (p.purpose.as_str(), p.tokens)).collect();
        assert_eq!(purposes, vec![("chat", 100), ("code", 40), ("title", 5)]);
        let modes: Vec<(&str, i64, i64)> =
            d.by_mode.iter().map(|m| (m.name.as_str(), m.events, m.tokens)).collect();
        assert_eq!(modes, vec![("chat", 2, 105), ("code", 1, 40)]);

        let chat = d.sessions_list.iter().find(|s| s.session_id == "chat-9").unwrap();
        assert_eq!(chat.mode, "chat");
        assert_eq!(chat.models, vec!["claude-sonnet-4-5"]);
        let code = d.sessions_list.iter().find(|s| s.session_id == "chat-1").unwrap();
        assert_eq!(code.mode, "code");

        let calls = wackcode_session_calls(&store, "chat-9").unwrap();
        assert_eq!(calls.iter().map(|c| c.purpose.as_str()).collect::<Vec<_>>(), vec!["chat", "title"]);
    }

    #[test]
    fn wackcode_session_calls_timeline_is_chronological_and_hidden_aware() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let base = 1_760_000_000_000_i64;
        let mut other_chat = wackcode_event("w9", base + 500, 999);
        other_chat.session_id = Some("chat-2".to_string());
        let mut cancelled = wackcode_event("w2", base + 2000, 40);
        cancelled.outcome = Some("cancelled".into());
        let mut sub = wackcode_event("w3", base + 1000, 60);
        sub.subagent_id = Some("sub-7".into());
        sub.is_subagent = true;
        store
            .insert_events(&[
                wackcode_event("w1", base, 10),
                other_chat,
                sub,
                cancelled,
            ])
            .unwrap();

        let calls = wackcode_session_calls(&store, "chat-1").unwrap();
        assert_eq!(calls.iter().map(|c| c.tokens).collect::<Vec<_>>(), vec![10, 60, 40]);
        assert_eq!(calls[1].subagent_id.as_deref(), Some("sub-7"));
        assert_eq!(calls[2].outcome, "cancelled");

        // hiding a model removes its calls from the timeline
        store.hide_models(&["claude-sonnet-4-5".to_string()]).unwrap();
        assert!(wackcode_session_calls(&store, "chat-1").unwrap().is_empty());
    }

    #[test]
    fn project_suggestions_offer_a_live_ancestor_for_a_nested_dead_folder() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        let root = crate::collectors::test_root().join(format!("tt-nest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let live = root.join("plain-project");
        std::fs::create_dir_all(&live).unwrap();
        let live_s = live.to_string_lossy().to_string();
        let gone_s = live.join("notes/scripts").to_string_lossy().to_string();

        store
            .insert_events(&[
                project_event("1", &gone_s, now - 2000, 300),
                project_event("2", &live_s, now - 1000, 900),
            ])
            .unwrap();

        let suggestions = project_suggestions(&store).unwrap();
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].from, gone_s);
        assert_eq!(suggestions[0].to, live_s);
        assert_eq!(suggestions[0].reason, "inside");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn project_suggestions_offer_a_live_subfolder_of_a_project() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        let root = crate::collectors::test_root().join(format!("tt-sub-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let project = root.join("plain-project");
        let sub = project.join("scripts");
        std::fs::create_dir_all(&sub).unwrap();
        let project_s = project.to_string_lossy().to_string();
        let sub_s = sub.to_string_lossy().to_string();

        store
            .insert_events(&[
                project_event("1", &project_s, now - 2000, 900),
                project_event("2", &sub_s, now - 1000, 100),
            ])
            .unwrap();

        let suggestions = project_suggestions(&store).unwrap();
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].from, sub_s);
        assert_eq!(suggestions[0].to, project_s);
        assert_eq!(suggestions[0].reason, "subfolder");
        assert!(suggestions[0].from_exists);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn project_suggestions_pair_a_dead_folder_with_a_live_namesake() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        let root = crate::collectors::test_root().join(format!("tt-sugg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let live = root.join("new/Probe");
        std::fs::create_dir_all(&live).unwrap();
        let live_s = live.to_string_lossy().to_string();
        let gone_s = root.join("old/Probe").to_string_lossy().to_string();

        store
            .insert_events(&[
                project_event("1", &gone_s, now - 2000, 500),
                project_event("2", &live_s, now - 1000, 100),
            ])
            .unwrap();

        let suggestions = project_suggestions(&store).unwrap();
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].from, gone_s);
        assert_eq!(suggestions[0].to, live_s);
        assert_eq!(suggestions[0].reason, "same_name");
        assert_eq!(suggestions[0].from_tokens, 500);

        // keeping the folder separate is the dismissal: the guess does not come back
        store.unmerge_projects(&[gone_s]).unwrap();
        assert!(project_suggestions(&store).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn project_folders_fold_into_one_project() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        let mk_repo = |name: &str| {
            let dir = crate::collectors::test_root().join(format!("tt-agg-{name}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join(".git")).unwrap();
            // Folders resolve to their real path, and temp_dir is a symlink on macOS.
            std::fs::canonicalize(&dir).unwrap()
        };
        let repo = mk_repo("agg-main");
        std::fs::create_dir_all(repo.join("sub/a")).unwrap();
        std::fs::create_dir_all(repo.join("sub/b")).unwrap();
        let repo_s = repo.to_string_lossy().to_string();
        let sub_a = repo.join("sub/a").to_string_lossy().to_string();
        let sub_b = repo.join("sub/b").to_string_lossy().to_string();
        let side = mk_repo("agg-side").to_string_lossy().to_string();
        let third = mk_repo("agg-third").to_string_lossy().to_string();

        store
            .insert_events(&[
                project_event("1", &sub_a, now - 5000, 100),
                project_event("2", &sub_b, now - 4000, 200),
                project_event("3", &repo_s, now - 3000, 50),
                project_event("4", &side, now - 2000, 10),
                project_event("5", &third, now - 1000, 5),
            ])
            .unwrap();

        // ingest folds each subfolder into its repository on the way in
        let rows = by_project(&store, 3650).unwrap();
        assert_eq!(rows.len(), 3);
        let repo_row = rows.iter().find(|r| r.project == repo_s).unwrap();
        assert_eq!(repo_row.tokens, 350);
        assert_eq!(repo_row.events, 3);
        assert_eq!(repo_row.sessions, 3);

        let daily = daily_by_project(&store, 3650).unwrap();
        assert_eq!(
            daily.iter().filter(|d| d.project == repo_s).map(|d| d.tokens).sum::<i64>(),
            350
        );

        // the model card's projects table folds the same way
        let md = model_detail(&store, "gpt-5", 3650).unwrap().unwrap();
        assert_eq!(md.by_project.len(), 3);
        assert_eq!(md.by_project[0].project, repo_s);
        assert_eq!(md.by_project[0].events, 3);

        // project detail covers every folded folder, and a link to a folded
        // folder answers for its project
        let pd = project_detail(&store, &repo_s, 3650).unwrap().unwrap();
        assert_eq!(pd.events, 3);
        assert_eq!(pd.tokens, 350);
        let pd2 = project_detail(&store, &sub_b, 3650).unwrap().unwrap();
        assert_eq!(pd2.project, repo_s);
        assert_eq!(pd2.events, 3);

        // achievements count projects, not folders — and the first project is the
        // repository even though its earliest event ran in a subfolder
        let achs = model_achievements(&store, "gpt-5").unwrap();
        let first = achs.iter().find(|a| a.kind == "first_project").unwrap();
        assert_eq!(first.value, repo_s);
        let explorer = achs.iter().find(|a| a.kind == "project_explorer").unwrap();
        assert_eq!(explorer.value, "Used in 3 projects");
    }

    #[test]
    fn hidden_models_excluded_everywhere() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        store
            .insert_events(&[
                test_event("codex-auto-review", now, 100),
                test_event("gpt-5", now - 1000, 200),
                UsageEvent {
                    model: None,
                    input_tokens: 50,
                    ..test_event("no-model", now - 2000, 0)
                },
            ])
            .unwrap();
        store.hide_models(&["codex-auto-review".into()]).unwrap();

        // Per-model views drop the hidden model; the NULL-model row stays.
        let rows = by_model(&store, 3650).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.model != "codex-auto-review"));
        assert!(rows.iter().any(|r| r.model == "unknown"));

        let stats = model_stats(&store, 3650).unwrap();
        assert_eq!(stats.len(), 2);
        assert!(stats.iter().all(|r| r.model != "codex-auto-review"));

        let dm = daily_by_model(&store, 3650).unwrap();
        assert!(dm.iter().all(|r| r.model != "codex-auto-review"));

        // Totals exclude the hidden model's tokens/events.
        let ov = overview(&store).unwrap();
        assert_eq!(ov.total_tokens, 250); // 200 gpt-5 + 50 no-model
        assert_eq!(ov.events, 2);
        assert_eq!(ov.by_source[0].tokens, 250);

        // Source/day/heatmap/hourly aggregates agree.
        let d = daily(&store, 3650).unwrap();
        assert_eq!(d.iter().map(|r| r.tokens).sum::<i64>(), 250);
        let hm = heatmap(&store, 3650).unwrap();
        assert_eq!(hm.iter().map(|r| r.tokens).sum::<i64>(), 250);
        let hr = hourly(&store).unwrap();
        assert_eq!(hr.iter().map(|r| r.tokens).sum::<i64>(), 250);

        // Unhiding brings the model back everywhere.
        store.unhide_model("codex-auto-review").unwrap();
        let rows = by_model(&store, 3650).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(overview(&store).unwrap().total_tokens, 350);
    }

    #[test]
    fn hiding_canonical_hides_merged_variants() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        store
            .insert_events(&[test_event("gpt-5-codex", now, 100), test_event("gpt-5", now - 1000, 200)])
            .unwrap();
        store.merge_models(&["gpt-5-codex".into(), "gpt-5".into()], "gpt-5").unwrap();

        // Hiding the display name hides every raw name merged into it.
        store.hide_models(&["gpt-5".into()]).unwrap();
        assert!(model_stats(&store, 3650).unwrap().is_empty());
        assert_eq!(overview(&store).unwrap().total_tokens, 0);

        store.unhide_model("gpt-5").unwrap();
        let stats = model_stats(&store, 3650).unwrap();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].tokens, 300);
    }

    /// NULL-model rows display as 'unknown'; hiding that display name must
    /// hide them. Compared against the raw name they can never match, which
    /// left an unhideable "unknown" model on every page.
    #[test]
    fn hiding_unknown_hides_null_model_rows() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        store
            .insert_events(&[
                test_event("gpt-5", now, 200),
                UsageEvent {
                    model: None,
                    input_tokens: 50,
                    ..test_event("no-model", now - 1000, 0)
                },
            ])
            .unwrap();
        store.hide_models(&["unknown".into()]).unwrap();

        let rows = by_model(&store, 3650).unwrap();
        assert!(rows.iter().all(|r| r.model != "unknown"));
        let ov = overview(&store).unwrap();
        assert_eq!(ov.total_tokens, 200);
        assert_eq!(ov.events, 1);

        // The leaderboard never announces a "first sighting" for it.
        let lb = leaderboard_events(&store, 3650).unwrap();
        assert!(lb.iter().all(|e| e.model != "unknown"));

        store.unhide_model("unknown").unwrap();
        assert!(by_model(&store, 3650).unwrap().iter().any(|r| r.model == "unknown"));
    }

    #[test]
    fn family_stats_groups_models() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        store
            .insert_events(&[
                test_event("claude-opus-5", now, 300),
                test_event("claude-sonnet-4.5", now - 1000, 200),
                test_event("gpt-5", now - 2000, 150),
                test_event("gemini-3-pro", now - 3000, 100),
            ])
            .unwrap();

        let families = family_stats(&store, 3650).unwrap();
        // Three families: Claude, GPT, Gemini — in descending token order.
        assert_eq!(families.len(), 3);
        assert_eq!(families[0].family, "Claude");
        assert_eq!(families[0].tokens, 500);
        assert_eq!(families[0].models.len(), 2);
        assert_eq!(families[1].family, "GPT");
        assert_eq!(families[1].tokens, 150);
        assert_eq!(families[2].family, "Gemini");
        assert_eq!(families[2].tokens, 100);
    }

    #[test]
    fn family_stats_respects_hidden_and_merged() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        store
            .insert_events(&[
                test_event("claude-opus-5", now, 300),
                test_event("claude-opus-5", now - 1000, 200),
                test_event("gpt-5", now - 2000, 150),
            ])
            .unwrap();
        // Merge the two Claude rows under the canonical name.
        store
            .merge_models(
                &["claude-opus-5".into(), "claude-opus-5".into()],
                "claude-opus-5",
            )
            .unwrap();

        let families = family_stats(&store, 3650).unwrap();
        assert_eq!(families[0].family, "Claude");
        assert_eq!(families[0].tokens, 500);
        assert_eq!(families[0].models.len(), 1);
        assert_eq!(families[0].models[0].events, 2);

        // Hiding the Claude display name removes it.
        store.hide_models(&["claude-opus-5".into()]).unwrap();
        let families = family_stats(&store, 3650).unwrap();
        assert!(families.iter().all(|f| f.family != "Claude"));
    }

    #[test]
    fn family_detail_aggregates_members_once() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        // Noon UTC yesterday: both events land on one calendar day whatever
        // time the test runs.
        let now = now_ms();
        let day = (now / 86_400_000 - 1) * 86_400_000 + 43_200_000;

        let mut opus = test_event("claude-opus-5", day, 300);
        opus.source = Source::ClaudeCode;
        opus.session_id = Some("s1".into());
        opus.project = Some("/dev/alpha".into());
        let mut sonnet = test_event("claude-sonnet-4.5", day + 1000, 200);
        sonnet.session_id = Some("s1".into());
        sonnet.project = Some("/dev/alpha".into());
        let mut gpt = test_event("gpt-5", day + 2000, 150);
        gpt.session_id = Some("s2".into());
        let mut unknown = test_event("totally-unknown-model", day + 3000, 70);
        unknown.session_id = Some("s3".into());

        store.insert_events(&[opus, sonnet, gpt, unknown]).unwrap();

        let claude = family_detail(&store, "claude", 3650).unwrap().unwrap();
        assert_eq!(claude.family, "Claude");
        assert_eq!(claude.tokens, 500);
        assert_eq!(claude.events, 2);
        // Two Claude models sharing a session count it once, unlike family_stats.
        assert_eq!(claude.sessions, 1);
        assert_eq!(claude.models.len(), 2);
        assert_eq!(claude.models[0].model, "claude-opus-5");
        assert_eq!(claude.by_source.len(), 2);
        assert_eq!(claude.by_project.len(), 1);
        assert_eq!(claude.by_project[0].project, "/dev/alpha");
        assert_eq!(claude.by_project[0].sessions, 1);
        assert_eq!(claude.active_days, 1);
        assert_eq!(claude.peak_day_tokens, 500);
        assert_eq!(claude.total_window_tokens, 720);

        // No prefix matches, so an unrecognised model is its own "Other" bucket.
        let other = family_detail(&store, "Other", 3650).unwrap().unwrap();
        assert_eq!(other.tokens, 70);
        assert_eq!(other.models.len(), 1);
    }

    #[test]
    fn family_detail_respects_range_and_hidden_models() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        let recent = now - 86_400_000;
        let old = now - 400 * 86_400_000;

        store
            .insert_events(&[
                test_event("claude-opus-5", recent, 100),
                test_event("claude-sonnet-4.5", old, 50),
            ])
            .unwrap();

        let window = family_detail(&store, "Claude", 90).unwrap().unwrap();
        assert_eq!(window.tokens, 100);
        assert_eq!(window.models.len(), 1);

        let all = family_detail(&store, "Claude", 3650).unwrap().unwrap();
        assert_eq!(all.tokens, 150);
        assert_eq!(all.models.len(), 2);

        store
            .hide_models(&["claude-opus-5".into(), "claude-sonnet-4.5".into()])
            .unwrap();
        assert!(family_detail(&store, "Claude", 3650).unwrap().is_none());
    }

    #[test]
    fn family_detail_unknown_or_unused_is_none() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        store
            .insert_events(&[test_event("gpt-5", now_ms(), 10)])
            .unwrap();

        assert!(family_detail(&store, "not-a-family", 3650).unwrap().is_none());
        assert!(family_detail(&store, "Kimi", 3650).unwrap().is_none());
        assert!(family_detail(&store, "gpt", 3650).unwrap().is_some());
    }

    #[test]
    fn family_achievements_roll_up_members() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        let day1 = (now / 86_400_000 - 3) * 86_400_000 + 43_200_000;
        let day2 = day1 + 86_400_000;
        let day3 = day2 + 86_400_000;

        let mut a = test_event("claude-opus-5", day1, 60_000_000);
        a.source = Source::ClaudeCode;
        a.project = Some("/dev/alpha".into());
        let mut b = test_event("claude-sonnet-4.5", day2, 45_000_000);
        b.project = Some("/dev/beta".into());
        let mut c = test_event("claude-haiku-4-5", day3, 5_000_000);
        c.project = Some("/dev/gamma".into());

        store.insert_events(&[a, b, c]).unwrap();

        let got = family_achievements(&store, "Claude").unwrap();
        let kinds: Vec<&str> = got.iter().map(|a| a.kind.as_str()).collect();
        assert!(kinds.contains(&"peak_day"));
        assert!(kinds.contains(&"longest_streak"));
        assert!(kinds.contains(&"multi_harness"));
        assert!(kinds.contains(&"family_champion"));
        assert!(kinds.contains(&"first_project"));
        assert!(kinds.contains(&"project_explorer"));

        // 60M then 45M crosses the 100M mark on the second day.
        let milestone = got
            .iter()
            .find(|a| a.kind == "token_milestone" && a.tier.as_deref() == Some("100m"))
            .expect("100M milestone");
        assert_eq!(milestone.earned_ts, Some(day2));

        let peak = got.iter().find(|a| a.kind == "peak_day").unwrap();
        assert_eq!(peak.value, "60M tokens");

        let streak = got.iter().find(|a| a.kind == "longest_streak").unwrap();
        assert_eq!(streak.value, "3 days in a row");

        // An unknown family has no achievements.
        assert!(family_achievements(&store, "nope").unwrap().is_empty());
    }

    #[test]
    fn model_detail_folds_aliases_and_breaks_down() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        store
            .insert_events(&[
                UsageEvent {
                    source: Source::Zcode,
                    project: Some("proj-alpha".into()),
                    input_tokens: 100,
                    output_tokens: 20,
                    ..test_event("GLM-5.3", now, 0)
                },
                UsageEvent {
                    source: Source::ClaudeCode,
                    project: Some("proj-alpha".into()),
                    input_tokens: 50,
                    output_tokens: 10,
                    ..test_event("glm-5.3", now - 1000, 0)
                },
                UsageEvent {
                    source: Source::Zcode,
                    project: Some("proj-beta".into()),
                    input_tokens: 30,
                    output_tokens: 5,
                    ..test_event("GLM-5.3", now - 86_400_000, 0)
                },
            ])
            .unwrap();

        // Before merging: "GLM-5.3" matches the two raw rows with that model name.
        let d = model_detail(&store, "GLM-5.3", 30).unwrap().unwrap();
        assert_eq!(d.tokens, 155); // (100+20) + (30+5)
        assert_eq!(d.events, 2);

        store
            .merge_models(&["GLM-5.3".into(), "glm-5.3".into()], "GLM-5.3")
            .unwrap();

        // After merging: both variants fold under canonical name.
        let d = model_detail(&store, "GLM-5.3", 30).unwrap().unwrap();
        assert_eq!(d.model, "GLM-5.3");
        assert_eq!(d.tokens, 215); // (100+20) + (50+10) + (30+5)
        assert_eq!(d.events, 3);
        assert_eq!(d.sessions, 0); // all events have session_id: None
        assert!(d.first_ts.is_some());
        assert!(d.last_ts.is_some());

        // Two distinct sources.
        assert_eq!(d.by_source.len(), 2);

        // Two distinct projects.
        assert_eq!(d.by_project.len(), 2);
        assert!(d.by_project.iter().any(|p| p.project == "proj-alpha"));
        assert!(d.by_project.iter().any(|p| p.project == "proj-beta"));

        // Active days: two events same day (now, now-1000), one day before → 2 days.
        assert!(d.active_days >= 1 && d.active_days <= 2);

        // Daily series has the same number of entries as active days.
        assert_eq!(d.daily.len(), d.active_days as usize);

        // Peak day has the highest tokens.
        assert!(d.peak_day_tokens > 0);
        assert_eq!(d.reasoning_tokens, 0);
    }

    #[test]
    fn model_detail_aggregates_reasoning_tokens() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        store
            .insert_events(&[
                UsageEvent {
                    source: Source::ClaudeCode,
                    input_tokens: 1000,
                    output_tokens: 500,
                    reasoning_tokens: Some(300),
                    ..test_event("claude-3-7-sonnet", now, 0)
                },
                UsageEvent {
                    source: Source::Codex,
                    input_tokens: 800,
                    output_tokens: 400,
                    reasoning_tokens: Some(200),
                    ..test_event("claude-3-7-sonnet", now - 1000, 0)
                },
                UsageEvent {
                    source: Source::Antigravity,
                    input_tokens: 600,
                    // Antigravity stores output_total - thinking in output_tokens
                    output_tokens: 150,
                    reasoning_tokens: Some(150),
                    ..test_event("claude-3-7-sonnet", now - 2000, 0)
                },
            ])
            .unwrap();

        let d = model_detail(&store, "claude-3-7-sonnet", 30).unwrap().unwrap();
        assert_eq!(d.input_tokens, 2400); // 1000 + 800 + 600
        // Total output: 500 + 400 + (150 + 150) = 1200
        assert_eq!(d.output_tokens, 1200);
        // Reasoning: 300 + 200 + 150 = 650
        assert_eq!(d.reasoning_tokens, 650);
        assert_eq!(d.events, 3);
    }

    #[test]
    fn model_detail_hidden_returns_none() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        store.insert_events(&[test_event("codex-auto-review", now, 100)]).unwrap();
        store.hide_models(&["codex-auto-review".into()]).unwrap();
        assert!(model_detail(&store, "codex-auto-review", 30).unwrap().is_none());
    }

    #[test]
    fn model_detail_unknown_model_returns_none() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        assert!(model_detail(&store, "nonexistent-model", 30).unwrap().is_none());
    }

    #[test]
    fn model_detail_respects_days_cutoff() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        let day_ms = 86_400_000i64;

        // Model A: today (100 tok), 10 days ago (200 tok), 50 days ago (300 tok)
        // Model B: today (50 tok)
        store
            .insert_events(&[
                UsageEvent {
                    source: Source::ClaudeCode,
                    input_tokens: 80,
                    output_tokens: 20,
                    ..test_event("model-a", now, 0)
                },
                UsageEvent {
                    source: Source::ClaudeCode,
                    input_tokens: 150,
                    output_tokens: 50,
                    ..test_event("model-a", now - 10 * day_ms, 0)
                },
                UsageEvent {
                    source: Source::ClaudeCode,
                    input_tokens: 250,
                    output_tokens: 50,
                    ..test_event("model-a", now - 50 * day_ms, 0)
                },
                UsageEvent {
                    source: Source::Zcode,
                    input_tokens: 40,
                    output_tokens: 10,
                    ..test_event("model-b", now, 0)
                },
            ])
            .unwrap();

        // 7-day range: only the event from today is visible
        let d7 = model_detail(&store, "model-a", 7).unwrap().unwrap();
        assert_eq!(d7.tokens, 100);
        assert_eq!(d7.events, 1);
        assert_eq!(d7.daily.len(), 1);
        // Window total includes model-a (100) + model-b (50)
        assert_eq!(d7.total_window_tokens, 150);

        // 30-day range: today + 10 days ago (100 + 200 = 300)
        let d30 = model_detail(&store, "model-a", 30).unwrap().unwrap();
        assert_eq!(d30.tokens, 300);
        assert_eq!(d30.events, 2);
        assert_eq!(d30.daily.len(), 2);
        assert_eq!(d30.total_window_tokens, 350);

        // 90-day range: all 3 events (100 + 200 + 300 = 600)
        let d90 = model_detail(&store, "model-a", 90).unwrap().unwrap();
        assert_eq!(d90.tokens, 600);
        assert_eq!(d90.events, 3);
        assert_eq!(d90.daily.len(), 3);
        assert_eq!(d90.total_window_tokens, 650);

        // A model that exists in DB but has 0 events in the selected window (model-b in 50-day window 2 days ago? model-b only has today's event, so let's check a window where it has none if its event was 10 days ago)
        // Let's test a model with no events in the last 5 days
        let store2 = Store::open(std::path::Path::new(":memory:")).unwrap();
        store2
            .insert_events(&[UsageEvent {
                source: Source::ClaudeCode,
                input_tokens: 80,
                output_tokens: 20,
                ..test_event("older-model", now - 10 * day_ms, 0)
            }])
            .unwrap();

        let d_empty = model_detail(&store2, "older-model", 5).unwrap().unwrap();
        assert_eq!(d_empty.model, "older-model");
        assert_eq!(d_empty.tokens, 0);
        assert_eq!(d_empty.events, 0);
        assert_eq!(d_empty.daily.len(), 0);
        assert_eq!(d_empty.by_source.len(), 0);
        assert_eq!(d_empty.by_project.len(), 0);
        assert_eq!(d_empty.active_days, 0);
    }

    #[test]
    fn by_project_and_model_detail_collapse_null_and_unknown_project() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        store
            .insert_events(&[
                UsageEvent {
                    source: Source::Zcode,
                    project: None,
                    input_tokens: 100,
                    output_tokens: 20,
                    ..test_event("gpt-5", now, 0)
                },
                UsageEvent {
                    source: Source::ClaudeCode,
                    project: Some("unknown".into()),
                    input_tokens: 50,
                    output_tokens: 10,
                    ..test_event("gpt-5", now - 1000, 0)
                },
            ])
            .unwrap();

        let proj = by_project(&store, 30).unwrap();
        assert_eq!(proj.len(), 1);
        assert_eq!(proj[0].project, "unknown");
        assert_eq!(proj[0].tokens, 180);
        assert_eq!(proj[0].events, 2);

        let detail = model_detail(&store, "gpt-5", 30).unwrap().unwrap();
        assert_eq!(detail.by_project.len(), 1);
        assert_eq!(detail.by_project[0].project, "unknown");
        assert_eq!(detail.by_project[0].tokens, 180);
        assert_eq!(detail.by_project[0].events, 2);
    }

    #[test]
    fn leaderboard_overtake_3_day_swap() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        let day_ms = 86_400_000i64;
        let day1 = now - 2 * day_ms;
        let day2 = now - 1 * day_ms;
        let day3 = now;

        // Day 1: Model A gets 1000, Model B gets 800
        // Day 2: Model B gets 300 (total 1100 > A's 1000 -> B overtakes A for #1, gap 100)
        // Day 3: Model A gets 200 (total 1200 > B's 1100 -> A overtakes B for #1, gap 100)
        store
            .insert_events(&[
                test_event("model-a", day1, 1000),
                test_event("model-b", day1, 800),
                test_event("model-b", day2, 300),
                test_event("model-a", day3, 200),
            ])
            .unwrap();

        let events = leaderboard_events(&store, 30).unwrap();
        let overtakes: Vec<_> = events.iter().filter(|e| e.kind == "overtake").collect();
        assert_eq!(overtakes.len(), 2);
        assert_eq!(overtakes[0].model, "model-a");
        assert_eq!(overtakes[0].other_model, Some("model-b".into()));
        assert_eq!(overtakes[0].rank, Some(1));
        assert_eq!(overtakes[0].tokens, 100);

        assert_eq!(overtakes[1].model, "model-b");
        assert_eq!(overtakes[1].other_model, Some("model-a".into()));
        assert_eq!(overtakes[1].rank, Some(1));
        assert_eq!(overtakes[1].tokens, 100);
    }

    #[test]
    fn leaderboard_overtake_beyond_top_20() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        let day_ms = 86_400_000i64;
        let day1 = now - 2 * day_ms;
        let day2 = now - 1 * day_ms;

        // Seed 25 models on day 1 with descending token counts
        // m01: 25000, m02: 24000, ... m15: 11000, m16: 10000, ... m21: 5000, m22: 4000, ... m25: 1000
        let mut events_day1 = Vec::new();
        for i in 1..=25 {
            let name = format!("m{:02}", i);
            let tokens = (26 - i) as i64 * 1000;
            events_day1.push(test_event(&name, day1, tokens));
        }
        store.insert_events(&events_day1).unwrap();

        // On day 2:
        // - m16 gets 1500 tokens -> total 11500, passing m15 (11000) for rank 15 (within top 20)
        // - m22 gets 1500 tokens -> total 5500, passing m21 (5000) for rank 21 (outside top 20)
        store
            .insert_events(&[
                test_event("m16", day2, 1500),
                test_event("m22", day2, 1500),
            ])
            .unwrap();

        let events = leaderboard_events(&store, 30).unwrap();
        let overtakes: Vec<_> = events.iter().filter(|e| e.kind == "overtake").collect();

        // Both m16's overtake into rank 15 and m22's overtake into rank 21 should be captured
        assert_eq!(overtakes.len(), 2);
        let m16_ot = overtakes.iter().find(|e| e.model == "m16").unwrap();
        assert_eq!(m16_ot.other_model, Some("m15".into()));
        assert_eq!(m16_ot.rank, Some(15));
        assert_eq!(m16_ot.tokens, 500);

        let m22_ot = overtakes.iter().find(|e| e.model == "m22").unwrap();
        assert_eq!(m22_ot.other_model, Some("m21".into()));
        assert_eq!(m22_ot.rank, Some(21));
        assert_eq!(m22_ot.tokens, 500);
    }

    #[test]
    fn leaderboard_record_gated_on_pre_window_history() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        let day_ms = 86_400_000i64;
        let pre_window_day = now - 50 * day_ms;
        let in_window_day1 = now - 10 * day_ms;
        let in_window_day2 = now - 9 * day_ms;
        let in_window_day3 = now - 8 * day_ms;

        // Model A has pre-window daily max of 1000 tokens
        // Model B has ONLY in-window events
        store
            .insert_events(&[
                test_event("model-a", pre_window_day, 1000),
                test_event("model-a", in_window_day1, 1500), // record day (1500 > 1000)
                test_event("model-a", in_window_day2, 1200), // not a record (1200 <= 1500)
                test_event("model-a", in_window_day3, 2000), // record day (2000 > 1500)
                test_event("model-b", in_window_day1, 5000), // no pre-window history -> skipped
            ])
            .unwrap();

        let events = leaderboard_events(&store, 30).unwrap();
        let records: Vec<_> = events.iter().filter(|e| e.kind == "record").collect();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].model, "model-a");
        assert_eq!(records[0].tokens, 2000);
        assert_eq!(records[1].model, "model-a");
        assert_eq!(records[1].tokens, 1500);

        // Model B should not have any record events
        assert!(!events.iter().any(|e| e.kind == "record" && e.model == "model-b"));
    }

    #[test]
    fn leaderboard_debut_vs_first_seen() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        let day_ms = 86_400_000i64;
        let pre_day = now - 40 * day_ms;
        let in_day1 = now - 10 * day_ms;
        let in_day2 = now - 5 * day_ms;

        // 5 established models in top 5
        store
            .insert_events(&[
                test_event("m1", pre_day, 10_000),
                test_event("m2", pre_day, 10_000),
                test_event("m3", pre_day, 10_000),
                test_event("m4", pre_day, 10_000),
                test_event("m5", pre_day, 10_000),
                // Model C first seen on in_day1 at rank 6 (below top 5)
                test_event("model-c", in_day1, 100),
                // Model C on in_day2 jumps into top 5 with prior history -> debut
                test_event("model-c", in_day2, 20_000),
                // Model D first seen on in_day2 with 50_000 (immediately #1, but first sighting, not debut)
                test_event("model-d", in_day2, 50_000),
            ])
            .unwrap();

        let events = leaderboard_events(&store, 30).unwrap();
        let debuts: Vec<_> = events.iter().filter(|e| e.kind == "debut").collect();
        assert_eq!(debuts.len(), 1);
        assert_eq!(debuts[0].model, "model-c");
        assert!(debuts[0].rank.unwrap() <= 5);

        let first_seens: Vec<_> = events.iter().filter(|e| e.kind == "first_seen").collect();
        assert!(first_seens.iter().any(|e| e.model == "model-c"));
        assert!(first_seens.iter().any(|e| e.model == "model-d"));
    }

    #[test]
    fn leaderboard_respects_hidden_and_alias() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        store
            .insert_events(&[
                test_event("raw-a1", now, 500),
                test_event("raw-a2", now, 600),
                test_event("hidden-x", now, 1000),
                test_event("unmerged-model", now, 400),
            ])
            .unwrap();

        store
            .merge_models(&["raw-a1".into(), "raw-a2".into()], "raw-a1")
            .unwrap();
        store.hide_models(&["hidden-x".into()]).unwrap();

        let events = leaderboard_events(&store, 30).unwrap();
        // hidden-x produces no events
        assert!(!events.iter().any(|e| e.model == "hidden-x"));
        // raw-a2 folded into raw-a1
        assert!(events.iter().all(|e| e.model != "raw-a2"));
        // merged/aliased model raw-a1 does NOT emit first_seen
        assert!(!events.iter().any(|e| e.kind == "first_seen" && e.model == "raw-a1"));
        // unmerged model DOES emit first_seen
        assert!(events.iter().any(|e| e.kind == "first_seen" && e.model == "unmerged-model"));
    }

    #[test]
    fn leaderboard_big6_promotion_and_demotion_with_tenure() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        let day_ms = 86_400_000i64;
        let day1 = now - 60 * day_ms;
        let day51 = now - 10 * day_ms;

        // Day 1: 6 models in top 6, plus m7 outside at rank 7
        store
            .insert_events(&[
                test_event("m1", day1, 60_000),
                test_event("m2", day1, 50_000),
                test_event("m3", day1, 40_000),
                test_event("m4", day1, 30_000),
                test_event("m5", day1, 20_000),
                test_event("m6", day1, 10_000),
                test_event("m7", day1, 5_000),
            ])
            .unwrap();

        // Day 51 (50 days later): m7 earns 10_000 more (total 15_000 > m6's 10_000)
        // m7 enters Big 6 at #6; m6 is demoted out of Big 6 after 50 days at #6.
        store
            .insert_events(&[test_event("m7", day51, 10_000)])
            .unwrap();

        // 1. On ALL filter (3650 days): Big 6 promotion & demotion should fire
        let all_events = leaderboard_events(&store, 3650).unwrap();

        let promotions: Vec<_> = all_events.iter().filter(|e| e.kind == "promotion").collect();
        assert_eq!(promotions.len(), 1);
        assert_eq!(promotions[0].model, "m7");
        assert_eq!(promotions[0].other_model, Some("m6".into()));
        assert_eq!(promotions[0].rank, Some(6));
        assert_eq!(promotions[0].tokens, 15_000);
        assert_eq!(promotions[0].tenure_days, None);

        let demotions: Vec<_> = all_events.iter().filter(|e| e.kind == "demotion").collect();
        assert_eq!(demotions.len(), 1);
        assert_eq!(demotions[0].model, "m6");
        assert_eq!(demotions[0].other_model, Some("m7".into()));
        assert_eq!(demotions[0].rank, Some(6));
        assert_eq!(demotions[0].tokens, 10_000);
        assert_eq!(demotions[0].tenure_days, Some(50));

        // Redundant overtake between m7 and m6 should be suppressed
        assert!(!all_events.iter().any(|e| e.kind == "overtake" && e.model == "m7" && e.other_model.as_deref() == Some("m6")));

        // Debut should be suppressed on ALL filter
        assert!(!all_events.iter().any(|e| e.kind == "debut"));

        // 2. On 30D filter: Big 6 promotion/demotion should NOT fire
        let win_events = leaderboard_events(&store, 30).unwrap();
        assert!(!win_events.iter().any(|e| e.kind == "promotion"));
        assert!(!win_events.iter().any(|e| e.kind == "demotion"));
    }

    #[test]
    fn leaderboard_debut_suppressed_after_pre_window_top5() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        let day_ms = 86_400_000i64;
        let pre_day = now - 40 * day_ms;
        let mid_day = now - 20 * day_ms;
        let reentry_day = now - 5 * day_ms;
        let control_day = now - 4 * day_ms;

        store
            .insert_events(&[
                // Pre-window: M sits at rank 5, with m1..m4 above it.
                test_event("m1", pre_day, 10_000),
                test_event("m2", pre_day, 9_000),
                test_event("m3", pre_day, 8_000),
                test_event("m4", pre_day, 7_000),
                test_event("M", pre_day, 6_000),
                test_event("tail", pre_day, 100),
                // Mid-window: four big models push M out of the top 5.
                test_event("m6", mid_day, 50_000),
                test_event("m7", mid_day, 40_000),
                test_event("m8", mid_day, 30_000),
                test_event("m9", mid_day, 20_000),
                // Control model N: first seen mid-window below the top 5...
                test_event("N", mid_day, 500),
                // ...then jumps in later -> debut expected.
                test_event("N", control_day, 59_000),
                // M re-enters the top 5: it was already a top-5 member once,
                // so this is not a debut no matter how far back that was.
                test_event("M", reentry_day, 60_000),
            ])
            .unwrap();

        let events = leaderboard_events(&store, 30).unwrap();
        let debuts: Vec<_> = events.iter().filter(|e| e.kind == "debut").collect();
        assert_eq!(debuts.len(), 1, "only the control model should debut");
        assert_eq!(debuts[0].model, "N");
        assert!(!events.iter().any(|e| e.kind == "debut" && e.model == "M"));
    }

    #[test]
    fn leaderboard_overtake_multi_rank_jump_lists_every_passed_model() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        let day_ms = 86_400_000i64;
        let day1 = now - 2 * day_ms;
        let day2 = now - day_ms;

        store
            .insert_events(&[
                test_event("a", day1, 1000),
                test_event("b", day1, 2000),
                test_event("c", day1, 3000),
                test_event("d", day1, 4000),
                test_event("e", day1, 5000),
                // a jumps from last to first in one day, passing all four.
                test_event("a", day2, 6000),
            ])
            .unwrap();

        let events = leaderboard_events(&store, 30).unwrap();
        let overtakes: Vec<_> = events.iter().filter(|e| e.kind == "overtake").collect();
        assert_eq!(overtakes.len(), 4);
        let others: Vec<_> = overtakes
            .iter()
            .map(|e| e.other_model.as_deref().unwrap())
            .collect();
        // Same-day overtakes are emitted in the passed model's previous-rank
        // order, deterministically.
        assert_eq!(others, ["e", "d", "c", "b"]);
        for ot in &overtakes {
            assert_eq!(ot.model, "a");
            assert_eq!(ot.rank, Some(1));
        }
        assert_eq!(overtakes[0].tokens, 2000); // 7000 - 5000
        assert_eq!(overtakes[1].tokens, 3000); // 7000 - 4000
        assert_eq!(overtakes[2].tokens, 4000); // 7000 - 3000
        assert_eq!(overtakes[3].tokens, 5000); // 7000 - 2000
    }

    #[test]
    fn leaderboard_tenure_counts_from_before_all_time_cutoff() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        let day_ms = 86_400_000i64;
        let ancient = now - 3700 * day_ms; // before the ALL filter's 3650-day cutoff
        let recent = now - day_ms;

        store
            .insert_events(&[
                test_event("m1", ancient, 60_000),
                test_event("m2", ancient, 50_000),
                test_event("m3", ancient, 40_000),
                test_event("m4", ancient, 30_000),
                test_event("m5", ancient, 20_000),
                test_event("m6", ancient, 10_000),
                test_event("m7", ancient, 5_000),
            ])
            .unwrap();

        // m7 passes m6 for #6; m6's tenure reaches back to the ancient day,
        // not to the cutoff.
        store.insert_events(&[test_event("m7", recent, 10_000)]).unwrap();

        let events = leaderboard_events(&store, 3650).unwrap();
        let demotions: Vec<_> = events.iter().filter(|e| e.kind == "demotion").collect();
        assert_eq!(demotions.len(), 1);
        assert_eq!(demotions[0].model, "m6");
        assert_eq!(demotions[0].tenure_days, Some(3699));
    }

    #[test]
    fn leaderboard_demotion_tenure_spans_whole_top6_run_not_last_rank() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        let day_ms = 86_400_000i64;
        let day1 = now - 90 * day_ms;
        let day50 = now - 40 * day_ms;
        let day90 = now;

        // Day 1: hero debuts at #1 and fills the top 6 with m2..m6; m7 sits at #7.
        store
            .insert_events(&[
                test_event("hero", day1, 99_000),
                test_event("m2", day1, 90_000),
                test_event("m3", day1, 80_000),
                test_event("m4", day1, 70_000),
                test_event("m5", day1, 60_000),
                test_event("m6", day1, 50_000),
                test_event("m7", day1, 5_000),
            ])
            .unwrap();

        // Day 50 (40 days before now): m2..m6 all pass hero, sliding it from
        // #1 to #6 without leaving the top 6 — the membership set is unchanged.
        store
            .insert_events(&[
                test_event("m2", day50, 50_000),
                test_event("m3", day50, 50_000),
                test_event("m4", day50, 50_000),
                test_event("m5", day50, 50_000),
                test_event("m6", day50, 50_000),
            ])
            .unwrap();

        // Day 90 (now): m7 reaches 100_000, tying m6 and edging hero (99_000)
        // out of the Big 6. hero's run lasted 90 days: day 1 -> today. If
        // tenure were measured from when it entered #6 it would read 40.
        store
            .insert_events(&[test_event("m7", day90, 95_000)])
            .unwrap();

        let events = leaderboard_events(&store, 3650).unwrap();

        let promotions: Vec<_> = events.iter().filter(|e| e.kind == "promotion").collect();
        assert_eq!(promotions.len(), 1);
        assert_eq!(promotions[0].model, "m7");
        assert_eq!(promotions[0].other_model, Some("hero".into()));

        let demotions: Vec<_> = events.iter().filter(|e| e.kind == "demotion").collect();
        assert_eq!(demotions.len(), 1);
        assert_eq!(demotions[0].model, "hero");
        assert_eq!(demotions[0].other_model, Some("m7".into()));
        assert_eq!(demotions[0].rank, Some(6));
        assert_eq!(demotions[0].tenure_days, Some(90));
    }

    #[test]
    fn model_big6_keeps_every_run_and_matches_demotion_tenure() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        let day_ms = 86_400_000i64;
        let day_a = now - 200 * day_ms;
        let day_b = now - 150 * day_ms;
        let day_c = now - 100 * day_ms;

        // A: hero sits at #5 of a full top 6; m7 and m8 trail outside it.
        store
            .insert_events(&[
                test_event("m2", day_a, 90_000),
                test_event("m3", day_a, 80_000),
                test_event("m4", day_a, 70_000),
                test_event("m5", day_a, 60_000),
                test_event("hero", day_a, 55_000),
                test_event("m6", day_a, 50_000),
                test_event("m7", day_a, 5_000),
                test_event("m8", day_a, 1_000),
            ])
            .unwrap();
        // B: m7 climbs to 65k and m6 to 60k, pushing hero (55k) out after 50 days.
        store
            .insert_events(&[test_event("m7", day_b, 60_000), test_event("m6", day_b, 10_000)])
            .unwrap();
        // C: hero storms back to #1, which drops m6 out after a 100-day run.
        store.insert_events(&[test_event("hero", day_c, 50_000)]).unwrap();

        let hero = model_big6(&store, "hero").unwrap().expect("hero made the Big 6");
        assert_eq!(hero.stints.len(), 2);
        assert_eq!(hero.stints[0].days, 50);
        assert!(hero.stints[0].exited.is_some());
        assert_eq!(hero.stints[1].exited, None, "hero is still in");
        assert_eq!(hero.stints[1].days, 100, "an open run counts up to today");
        assert_eq!(hero.total_days, 150);
        assert_eq!(hero.longest_days, 100);

        // The finished run is the same number the exit event showed.
        let events = leaderboard_events(&store, 3650).unwrap();
        let m6_exit = events
            .iter()
            .find(|e| e.kind == "demotion" && e.model == "m6")
            .expect("m6 exit event");
        let m6 = model_big6(&store, "m6").unwrap().unwrap();
        assert_eq!(m6.stints.len(), 1);
        assert_eq!(Some(m6.stints[0].days), m6_exit.tenure_days);
        assert_eq!(m6.stints[0].exited.as_deref(), Some(m6_exit.date.as_str()));

        assert_eq!(model_big6(&store, "m8").unwrap(), None, "never made the top 6");
        assert_eq!(model_big6(&store, "nope").unwrap(), None);
    }

    #[test]
    fn big6_timeline_tiles_seats_and_sums_to_membership() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        let day_ms = 86_400_000i64;
        let day1 = now - 90 * day_ms;
        let day50 = now - 40 * day_ms;

        // Day 1: hero takes #1, m2..m6 fill the rest; m7 sits outside.
        store
            .insert_events(&[
                test_event("hero", day1, 99_000),
                test_event("m2", day1, 90_000),
                test_event("m3", day1, 80_000),
                test_event("m4", day1, 70_000),
                test_event("m5", day1, 60_000),
                test_event("m6", day1, 50_000),
                test_event("m7", day1, 5_000),
            ])
            .unwrap();

        // Day 50: m2..m6 all pass hero — it slides #1 → #6, closing its #1 run
        // and opening a #6 run, without ever leaving the Big 6.
        store
            .insert_events(&[
                test_event("m2", day50, 50_000),
                test_event("m3", day50, 50_000),
                test_event("m4", day50, 50_000),
                test_event("m5", day50, 50_000),
                test_event("m6", day50, 50_000),
            ])
            .unwrap();

        // Today: m7 edges hero out of the Big 6.
        store.insert_events(&[test_event("m7", now, 95_000)]).unwrap();

        let tl = big6_timeline(&store).unwrap();
        assert_eq!(tl.seats.len(), 6);

        // Once a seat is filled its runs tile contiguously: each run's exit
        // is the next run's entry, and only the last can still be open.
        for seat in &tl.seats {
            assert!(!seat.runs.is_empty(), "seat #{} never filled", seat.rank);
            for pair in seat.runs.windows(2) {
                assert_eq!(pair[0].exited.as_deref(), Some(pair[1].entered.as_str()));
            }
            assert!(seat.runs[..seat.runs.len() - 1]
                .iter()
                .all(|r| r.exited.is_some()));
            assert_eq!(seat.runs.last().unwrap().exited, None);
        }

        // Hero's seat days: 50d at #1 then 40d at #6 = 90d in the Big 6, the
        // membership total model_big6 reports and the demotion tenure showed.
        let hero = tl
            .seats
            .iter()
            .flat_map(|s| s.runs.iter())
            .filter(|r| r.model == "hero")
            .collect::<Vec<_>>();
        assert_eq!(hero.len(), 2);
        assert_eq!(hero[0].days, 50);
        assert_eq!(hero[1].days, 40);
        assert_eq!(hero.iter().map(|r| r.days).sum::<i64>(), 90);
        assert_eq!(
            model_big6(&store, "hero").unwrap().unwrap().total_days,
            90
        );

        // m6 sat at #6 the whole run — its seat days equal its demotion tenure.
        let m6_days: i64 = tl
            .seats
            .iter()
            .flat_map(|s| s.runs.iter())
            .filter(|r| r.model == "m6")
            .map(|r| r.days)
            .sum();
        assert_eq!(model_big6(&store, "m6").unwrap().unwrap().total_days, m6_days);
    }

    #[test]
    fn big6_timeline_fills_seats_progressively_and_respects_hidden_alias() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        let day_ms = 86_400_000i64;
        let day1 = now - 30 * day_ms;
        let day2 = now - 20 * day_ms;
        let day3 = now - 10 * day_ms;

        store.hide_models(&["hidden-m".into()]).unwrap();
        store.rename_model("raw-flash", "flash-canon").unwrap();
        store
            .insert_events(&[
                test_event("alpha", day1, 5_000),
                test_event("beta", day1, 3_000),
                test_event("gamma", day1, 1_000),
            ])
            .unwrap();
        // Hidden rows never reach the standings; the aliased model shows
        // under its canonical name and jumps straight to #1.
        store
            .insert_events(&[
                test_event("hidden-m", day2, 999_000),
                test_event("raw-flash", day2, 10_000),
            ])
            .unwrap();
        store
            .insert_events(&[test_event("alpha", day3, 10_000)])
            .unwrap();

        let tl = big6_timeline(&store).unwrap();

        // Four seen models fill seats #1–#4 only; #5 and #6 stay empty.
        assert!(tl.seats[4].runs.is_empty() && tl.seats[5].runs.is_empty());
        assert!(tl.seats[..4].iter().all(|s| !s.runs.is_empty()));

        // #1: alpha (day1→day2), raw-flash shown as flash-canon (day2→day3),
        // alpha again (day3→now, 10d so far).
        let s1 = &tl.seats[0].runs;
        assert_eq!(s1.len(), 3);
        assert_eq!(s1[0].model, "alpha");
        assert!(s1[0].exited.is_some());
        assert_eq!(s1[1].model, "flash-canon");
        assert!(s1[1].exited.is_some());
        assert_eq!(s1[2].model, "alpha");
        assert_eq!(s1[2].exited, None);
        assert_eq!(s1[2].days, 10);

        // Neither the hidden raw name nor the alias string appears anywhere.
        assert!(!tl
            .seats
            .iter()
            .flat_map(|s| s.runs.iter())
            .any(|r| r.model == "hidden-m" || r.model == "raw-flash"));
        // The canonical name does land as a real member.
        assert_eq!(
            model_big6(&store, "flash-canon")
                .unwrap()
                .unwrap()
                .total_days,
            20
        );
    }

    #[test]
    fn project_detail_returns_none_for_nonexistent_project() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        assert!(project_detail(&store, "nonexistent-proj", 30).unwrap().is_none());
        assert!(project_detail(&store, "unknown", 30).unwrap().is_none());
    }

    #[test]
    fn project_detail_aggregates_project_and_sessions() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        store
            .insert_events(&[
                UsageEvent {
                    source: Source::ClaudeCode,
                    source_event_id: "cc-1".into(),
                    project: Some("/path/to/my-app".into()),
                    session_id: Some("sess-1".into()),
                    input_tokens: 500,
                    output_tokens: 100,
                    ..test_event("claude-3-7-sonnet", now - 5000, 500)
                },
                UsageEvent {
                    source: Source::ClaudeCode,
                    source_event_id: "cc-2".into(),
                    project: Some("/path/to/my-app".into()),
                    session_id: Some("sess-1".into()),
                    input_tokens: 400,
                    output_tokens: 50,
                    ..test_event("gpt-4o", now - 4000, 400)
                },
                UsageEvent {
                    source: Source::Antigravity,
                    source_event_id: "ag-1".into(),
                    project: Some("/path/to/my-app".into()),
                    session_id: Some("sess-2".into()),
                    input_tokens: 300,
                    output_tokens: 200,
                    reasoning_tokens: Some(50),
                    ..test_event("gemini-2.5-pro", now - 1000, 300)
                },
                UsageEvent {
                    source: Source::Zcode,
                    source_event_id: "zc-other".into(),
                    project: Some("/path/to/other".into()),
                    session_id: Some("sess-other".into()),
                    input_tokens: 1000,
                    output_tokens: 200,
                    ..test_event("gpt-5", now, 1000)
                },
            ])
            .unwrap();

        let detail = project_detail(&store, "/path/to/my-app", 30).unwrap().unwrap();
        assert_eq!(detail.project, "/path/to/my-app");
        assert_eq!(detail.events, 3);
        assert_eq!(detail.sessions, 2);
        assert_eq!(detail.by_source.len(), 2);
        assert_eq!(detail.by_model.len(), 3);
        assert_eq!(detail.sessions_list.len(), 2);

        // sessions_list is ordered by last_ts DESC
        assert_eq!(detail.sessions_list[0].session_id, "sess-2");
        assert_eq!(detail.sessions_list[0].events, 1);
        assert_eq!(detail.sessions_list[0].models, vec!["gemini-2.5-pro"]);

        assert_eq!(detail.sessions_list[1].session_id, "sess-1");
        assert_eq!(detail.sessions_list[1].events, 2);
        assert_eq!(detail.sessions_list[1].models, vec!["claude-3-7-sonnet", "gpt-4o"]);

        // total window tokens includes /path/to/other (1200) + 1550 = 2750
        assert_eq!(detail.total_window_tokens, 2750);
    }

    #[test]
    fn project_detail_handles_unknown_and_null_project() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        store
            .insert_events(&[
                UsageEvent {
                    source: Source::Zcode,
                    source_event_id: "zc-1".into(),
                    project: None,
                    session_id: Some("sess-u1".into()),
                    input_tokens: 100,
                    output_tokens: 20,
                    ..test_event("gpt-5", now - 2000, 100)
                },
                UsageEvent {
                    source: Source::ClaudeCode,
                    source_event_id: "cc-1".into(),
                    project: Some("unknown".into()),
                    session_id: Some("sess-u2".into()),
                    input_tokens: 200,
                    output_tokens: 40,
                    ..test_event("gpt-5", now - 1000, 200)
                },
            ])
            .unwrap();

        let detail = project_detail(&store, "unknown", 30).unwrap().unwrap();
        assert_eq!(detail.project, "unknown");
        assert_eq!(detail.events, 2);
        assert_eq!(detail.sessions, 2);
        assert_eq!(detail.tokens, 360);
        assert_eq!(detail.sessions_list.len(), 2);
    }

    #[test]
    fn project_detail_respects_hidden_and_alias() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        store
            .insert_events(&[
                UsageEvent {
                    source: Source::Zcode,
                    source_event_id: "zc-1".into(),
                    project: Some("proj-x".into()),
                    session_id: Some("s1".into()),
                    input_tokens: 100,
                    output_tokens: 20,
                    ..test_event("raw-mod-1", now, 100)
                },
                UsageEvent {
                    source: Source::Zcode,
                    source_event_id: "zc-2".into(),
                    project: Some("proj-x".into()),
                    session_id: Some("s1".into()),
                    input_tokens: 150,
                    output_tokens: 30,
                    ..test_event("raw-mod-2", now, 150)
                },
                UsageEvent {
                    source: Source::Zcode,
                    source_event_id: "zc-3".into(),
                    project: Some("proj-x".into()),
                    session_id: Some("s2".into()),
                    input_tokens: 500,
                    output_tokens: 100,
                    ..test_event("hidden-mod", now, 500)
                },
            ])
            .unwrap();

        store.merge_models(&["raw-mod-1".into(), "raw-mod-2".into()], "raw-mod-1").unwrap();
        store.hide_models(&["hidden-mod".into()]).unwrap();

        let detail = project_detail(&store, "proj-x", 30).unwrap().unwrap();
        assert_eq!(detail.events, 2); // hidden-mod excluded
        assert_eq!(detail.by_model.len(), 1);
        assert_eq!(detail.by_model[0].model, "raw-mod-1");
        assert_eq!(detail.by_model[0].tokens, 300);
        assert_eq!(detail.sessions_list.len(), 1);
        assert_eq!(detail.sessions_list[0].models, vec!["raw-mod-1"]);
    }

    #[test]
    fn peak_days_ranking_and_filters() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        let day_ms = 86_400_000_i64;

        let events = vec![
            test_event("model-a", now, 1000),
            test_event("model-a", now - day_ms, 5000),
            test_event("model-b", now, 3000),
            test_event("model-b", now - day_ms, 2000),
            test_event("model-c", now - 2 * day_ms, 4000),
            test_event("model-d", now - 2 * day_ms, 1500),
            test_event("model-e", now - 40 * day_ms, 9000),
            test_event("secret-model", now, 99999),
            test_event("model-a-variant", now, 600),
        ];
        store.insert_events(&events).unwrap();
        store
            .merge_models(&["model-a".into(), "model-a-variant".into()], "model-a")
            .unwrap();
        store.hide_models(&["secret-model".into()]).unwrap();

        let all_time = peak_days(&store, 0).unwrap();
        assert_eq!(all_time.len(), 5);
        assert_eq!(all_time[0].model, "model-e");
        assert_eq!(all_time[0].tokens, 9000);
        assert_eq!(all_time[1].model, "model-a");
        assert_eq!(all_time[1].tokens, 5000);
        assert_eq!(all_time[2].model, "model-c");
        assert_eq!(all_time[2].tokens, 4000);
        assert_eq!(all_time[3].model, "model-b");
        assert_eq!(all_time[3].tokens, 3000);
        assert_eq!(all_time[4].model, "model-b");
        assert_eq!(all_time[4].tokens, 2000);

        let last_30 = peak_days(&store, 30).unwrap();
        assert_eq!(last_30.len(), 5);
        assert_eq!(last_30[0].model, "model-a");
        assert_eq!(last_30[0].tokens, 5000);
        assert_eq!(last_30[1].model, "model-c");
        assert_eq!(last_30[1].tokens, 4000);
        assert_eq!(last_30[2].model, "model-b");
        assert_eq!(last_30[2].tokens, 3000);
        assert_eq!(last_30[3].model, "model-b");
        assert_eq!(last_30[3].tokens, 2000);
        assert_eq!(last_30[4].model, "model-a");
        assert_eq!(last_30[4].tokens, 1600);
    }

    #[test]
    fn model_achievements_unknown_or_hidden_returns_empty() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        store
            .insert_events(&[
                test_event("secret-model", now, 5000),
            ])
            .unwrap();
        store.hide_models(&["secret-model".into()]).unwrap();

        assert!(model_achievements(&store, "nonexistent-model").unwrap().is_empty());
        assert!(model_achievements(&store, "secret-model").unwrap().is_empty());
    }

    #[test]
    fn model_achievements_computes_all_kinds() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let day_ms = 86_400_000i64;
        let day1 = 1700000000000i64; // arbitrary fixed epoch ms
        let day2 = day1 + day_ms;
        let day3 = day2 + day_ms;

        let mut ev1 = test_event("claude-sonnet-4.5", day1, 80_000);
        ev1.source = Source::Zcode;
        ev1.project = Some("proj-alpha".into());

        let mut ev2 = test_event("claude-sonnet-4.5", day2, 50_000);
        ev2.source = Source::ClaudeCode;
        ev2.project = Some("proj-beta".into());

        let mut ev3 = test_event("claude-sonnet-4.5", day3, 1_100_000_000);
        ev3.source = Source::Zcode;
        ev3.project = Some("proj-gamma".into());

        let ev_haiku = test_event("claude-haiku-4", day1, 10_000);

        store
            .insert_events(&[ev1, ev2, ev3, ev_haiku])
            .unwrap();

        let achs = model_achievements(&store, "claude-sonnet-4.5").unwrap();

        // 1. Peak day
        let peak = achs.iter().find(|a| a.kind == "peak_day").expect("peak_day present");
        assert_eq!(peak.title, "Peak Day");
        assert_eq!(peak.earned_ts, Some(day3));
        assert!(peak.value.contains("1.1B") || peak.value.contains("tokens"));

        // 2. Longest streak (3 days: day1, day2, day3)
        let streak = achs.iter().find(|a| a.kind == "longest_streak").expect("longest_streak present");
        assert_eq!(streak.value, "3 days in a row");
        assert_eq!(streak.earned_ts, Some(day3));

        // 3. Multi-harness (ZCode on day1, ClaudeCode on day2)
        let multi = achs.iter().find(|a| a.kind == "multi_harness").expect("multi_harness present");
        assert_eq!(multi.value, "Used in 2 harnesses");
        assert_eq!(multi.earned_ts, Some(day2));

        // 4. Family champion
        let champ = achs.iter().find(|a| a.kind == "family_champion").expect("family_champion present");
        assert_eq!(champ.value, "Top model in Claude");
        assert_eq!(champ.earned_ts, None);

        // 5. Token milestones: 100m, 500m and 1b all on day3 (130k + 1.1B crosses every tier)
        let m100m = achs.iter().find(|a| a.kind == "token_milestone" && a.tier.as_deref() == Some("100m")).expect("100m milestone");
        assert_eq!(m100m.earned_ts, Some(day3));

        let m500m = achs.iter().find(|a| a.kind == "token_milestone" && a.tier.as_deref() == Some("500m")).expect("500m milestone");
        assert_eq!(m500m.earned_ts, Some(day3));

        let m1b = achs.iter().find(|a| a.kind == "token_milestone" && a.tier.as_deref() == Some("1b")).expect("1b milestone");
        assert_eq!(m1b.earned_ts, Some(day3));

        // The retired easy tiers are gone.
        assert!(!achs.iter().any(|a| a.kind == "token_milestone"
            && matches!(a.tier.as_deref(), Some("100k" | "1m" | "10m"))));

        // 6. First project & Project explorer
        let first_proj = achs.iter().find(|a| a.kind == "first_project").expect("first_project present");
        assert_eq!(first_proj.title, "First Project");
        assert_eq!(first_proj.value, "proj-alpha");
        assert_eq!(first_proj.earned_ts, Some(day1));

        let proj = achs.iter().find(|a| a.kind == "project_explorer").expect("project_explorer present");
        assert_eq!(proj.value, "Used in 3 projects");
        assert_eq!(proj.earned_ts, Some(day3));
    }

    #[test]
    fn model_achievements_without_project_has_no_first_project() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        store
            .insert_events(&[
                test_event("gpt-5", now, 5000),
            ])
            .unwrap();

        let achs = model_achievements(&store, "gpt-5").unwrap();
        assert!(!achs.iter().any(|a| a.kind == "first_project"));
    }

    #[test]
    fn daily_by_project_excludes_unknown_and_aggregates_properly() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let day_ms = 86_400_000i64;
        let day1 = 1700000000000i64; // arbitrary fixed epoch ms: 2023-11-14
        let day2 = day1 + day_ms;

        let mut ev1 = test_event("gpt-4", day1, 100);
        ev1.project = Some("project-a".into());

        let mut ev2 = test_event("gpt-4", day1 + 1000, 200);
        ev2.project = Some("project-a".into());

        let mut ev3 = test_event("gpt-4", day1 + 2000, 300);
        ev3.project = Some("project-b".into());

        let mut ev_unknown1 = test_event("gpt-4", day1 + 3000, 400);
        ev_unknown1.project = None;

        let mut ev_unknown2 = test_event("gpt-4", day1 + 4000, 500);
        ev_unknown2.project = Some("unknown".into());

        let mut ev_unknown3 = test_event("gpt-4", day1 + 5000, 600);
        ev_unknown3.project = Some("".into());

        let mut ev_day2 = test_event("gpt-4", day2, 700);
        ev_day2.project = Some("project-b".into());

        store
            .insert_events(&[
                ev1, ev2, ev3, ev_unknown1, ev_unknown2, ev_unknown3, ev_day2,
            ])
            .unwrap();

        let rows = daily_by_project(&store, 0).unwrap();
        // Day 1 has project-a (100 + 200 = 300) and project-b (300). Unknowns are excluded.
        // Day 2 has project-b (700).
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].project, "project-a");
        assert_eq!(rows[0].tokens, 300);

        assert_eq!(rows[1].project, "project-b");
        assert_eq!(rows[1].tokens, 300);

        assert_eq!(rows[2].project, "project-b");
        assert_eq!(rows[2].tokens, 700);
        assert_ne!(rows[0].date, rows[2].date);
    }

    #[test]
    fn model_stats_and_active_days_for_range() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let day1 = 1_700_000_000_000;
        let day2 = day1 + 86_400_000;

        store
            .insert_events(&[
                test_event("gpt-4", day1, 100),
                test_event("gpt-4", day1 + 5000, 200),
                test_event("gpt-4", day2, 300),
                test_event("claude-3", day2 + 1000, 400),
            ])
            .unwrap();

        let stats = model_stats(&store, 3650).unwrap();
        let gpt4 = stats.iter().find(|r| r.model == "gpt-4").unwrap();
        assert_eq!(gpt4.active_days, 2);
        assert_eq!(gpt4.events, 3);
        assert_eq!(gpt4.tokens, 600);

        let claude = stats.iter().find(|r| r.model == "claude-3").unwrap();
        assert_eq!(claude.active_days, 1);
        assert_eq!(claude.events, 1);
        assert_eq!(claude.tokens, 400);

        let active_days_all = active_days_for_range(&store, 3650).unwrap();
        assert_eq!(active_days_all, 2);
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
    }

    fn utc_date(ts: i64) -> String {
        time::OffsetDateTime::from_unix_timestamp(ts / 1000)
            .unwrap()
            .date()
            .format(&format_description!("[year]-[month]-[day]"))
            .unwrap()
    }

    #[test]
    fn model_comparison_exact_cost_and_equivalence_use_stored_costs() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        store
            .insert_events(&[
                wrapped_event("cost-a", "gpt-5", now, 1_000_000, None, None, false),
                wrapped_event("cost-b", "claude-sonnet-4.5", now, 1_000_000, None, None, false),
            ])
            .unwrap();

        let cmp = model_comparison(&store, "gpt-5", "claude-sonnet-4.5", 0, 2_000_000)
            .unwrap()
            .unwrap();
        assert_eq!(cmp.a.priced_tokens, 1_000_000);
        assert_eq!(cmp.b.priced_tokens, 1_000_000);
        assert_close(cmp.a.priced_cost_usd.unwrap(), 1.25);
        assert_close(cmp.b.priced_cost_usd.unwrap(), 3.0);
        assert_close(cmp.a.cost_per_mtok.unwrap(), 1.25);
        assert_close(cmp.b.cost_per_mtok.unwrap(), 3.0);
        let cost = cmp.cost_comparison.unwrap();
        assert_eq!(cost.anchor_tokens, 2_000_000);
        assert_close(cost.a_anchor_cost_usd, 2.5);
        assert_close(cost.b_anchor_cost_usd, 6.0);
        assert_close(cost.b_equivalent_tokens.unwrap(), 2_000_000.0 * 1.25 / 3.0);

        // Comparison reprices nothing: an existing historical cost is the input.
        store
            .read_conn()
            .execute("UPDATE usage_event SET cost_usd=2.0 WHERE source_event_id='cost-a'", [])
            .unwrap();
        let cmp = model_comparison(&store, "gpt-5", "claude-sonnet-4.5", 0, 1_000_000)
            .unwrap()
            .unwrap();
        let cost = cmp.cost_comparison.unwrap();
        assert_close(cost.a_anchor_cost_usd, 2.0);
        assert_close(cost.b_equivalent_tokens.unwrap(), 1_000_000.0 * 2.0 / 3.0);
    }

    #[test]
    fn model_comparison_distinguishes_free_from_unpriced() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        store
            .insert_events(&[
                wrapped_event("free", "deepseek-v4-flash-free", now, 1000, None, None, false),
                wrapped_event("paid", "gpt-5", now, 1000, None, None, false),
                wrapped_event("unpriced", "never-priced-model", now, 2000, None, None, false),
            ])
            .unwrap();

        let cmp = model_comparison(&store, "deepseek-v4-flash-free", "gpt-5", 0, 1_000_000)
            .unwrap()
            .unwrap();
        assert_eq!(cmp.a.priced_events, 1);
        assert_eq!(cmp.a.priced_tokens, 1000);
        assert_eq!(cmp.a.priced_cost_usd, Some(0.0));
        assert_eq!(cmp.a.cost_per_mtok, Some(0.0));
        let cost = cmp.cost_comparison.unwrap();
        assert_eq!(cost.a_anchor_cost_usd, 0.0);
        assert_close(cost.b_anchor_cost_usd, 1.25);
        assert_eq!(cost.b_equivalent_tokens, None);

        let cmp = model_comparison(
            &store,
            "deepseek-v4-flash-free",
            "never-priced-model",
            0,
            1_000_000,
        )
        .unwrap()
        .unwrap();
        assert_eq!(cmp.a.cost_per_mtok, Some(0.0));
        assert_eq!(cmp.b.measured_tokens, 2000);
        assert_eq!(cmp.b.priced_tokens, 0);
        assert_eq!(cmp.b.priced_events, 0);
        assert_eq!(cmp.b.priced_cost_usd, None);
        assert_eq!(cmp.b.cost_per_mtok, None);
        assert!(cmp.cost_comparison.is_none());
    }

    #[test]
    fn model_comparison_estimated_unpriced_and_breakdown_coverage() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        let priced = wrapped_event("cover-1", "gpt-5", now, 1000, Some("shared"), Some("/raw/a"), false);
        let unpriced = wrapped_event("cover-2", "gpt-5", now + 1, 500, Some("shared"), Some("/raw/a"), false);
        let mut estimated = wrapped_event("cover-3", "gpt-5", now + 2, 300, Some("shared"), Some("/raw/a"), true);
        estimated.is_subagent = true;
        let mut other_source = wrapped_event("cover-4", "gpt-5", now + 3, 2000, Some("shared"), Some("/raw/b"), false);
        other_source.source = Source::ClaudeCode;
        let control = wrapped_event("cover-b", "coverage-control", now, 7, None, None, false);
        store.insert_events(&[priced, unpriced, estimated, other_source, control]).unwrap();
        store
            .read_conn()
            .execute("UPDATE usage_event SET cost_usd=NULL WHERE source_event_id='cover-2'", [])
            .unwrap();
        let estimated_cost: Option<f64> = store
            .read_conn()
            .query_row("SELECT cost_usd FROM usage_event WHERE source_event_id='cover-3'", [], |r| r.get(0))
            .unwrap();
        assert!(estimated_cost.is_some(), "the live ingest path can leave a cost on an estimate");
        {
            let conn = store.read_conn();
            for alias in ["/raw/a", "/raw/b"] {
                conn.execute(
                    "INSERT INTO project_alias(alias,canonical) VALUES(?1,'/repo')",
                    [alias],
                )
                .unwrap();
            }
        }

        let cmp = model_comparison(&store, "gpt-5", "coverage-control", 0, 1000)
            .unwrap()
            .unwrap();
        let a = &cmp.a;
        assert_eq!(a.tokens, 3800);
        assert_eq!(a.measured_tokens, 3500);
        assert_eq!(a.estimated_tokens, 300);
        assert_eq!(a.priced_tokens, 3000);
        assert_eq!(a.priced_events, 2);
        assert_close(a.priced_cost_usd.unwrap(), 0.00375);
        assert_close(a.cost_per_mtok.unwrap(), 1.25);
        assert_eq!(a.sessions, 2, "the same session id in two sources is two sessions");
        assert_eq!(a.subagent_tokens, 300);
        assert_eq!(a.hourly.iter().map(|h| h.tokens).sum::<i64>(), 3800);

        let zcode = a.by_source.iter().find(|s| s.source == "zcode").unwrap();
        assert_eq!((zcode.tokens, zcode.events, zcode.sessions), (1800, 3, 1));
        assert_close(zcode.cost_usd.unwrap(), 0.00125);
        let claude = a.by_source.iter().find(|s| s.source == "claude_code").unwrap();
        assert_close(claude.cost_usd.unwrap(), 0.0025);
        assert_eq!(a.by_project.len(), 1);
        let project = &a.by_project[0];
        assert_eq!(project.project, "/repo");
        assert_eq!((project.tokens, project.events, project.sessions), (3800, 4, 2));
        assert_close(project.cost_usd.unwrap(), 0.00375);
        assert_eq!(cmp.total_window_tokens, 3807);
    }

    #[test]
    fn model_comparison_aliases_dated_names_hidden_and_qualified_sessions() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let cut = now_ms() - 2 * 86_400_000;
        let old = wrapped_event("alias-old", "float", cut - 1000, 100, Some("shared"), None, false);
        let new = wrapped_event("alias-new", "float", cut + 1000, 40, Some("shared"), None, false);
        let mut variant = wrapped_event("alias-variant", "variant", cut, 50, Some("shared"), None, false);
        variant.source = Source::ClaudeCode;
        store.insert_events(&[old, new, variant]).unwrap();
        store.rename_model("float", "Old Display").unwrap();
        store.split_model_alias("float", cut, "New Display").unwrap();
        store.rename_model("variant", "Old Display").unwrap();

        let cmp = model_comparison(&store, "Old Display", "New Display", 0, 1000)
            .unwrap()
            .unwrap();
        assert_eq!(cmp.a.model, "Old Display");
        assert_eq!(cmp.a.tokens, 150);
        assert_eq!(cmp.a.sessions, 2);
        assert_eq!(cmp.b.model, "New Display");
        assert_eq!(cmp.b.tokens, 40);
        assert_eq!(cmp.daily.iter().map(|d| d.a_tokens).sum::<i64>(), 150);
        assert_eq!(cmp.daily.iter().map(|d| d.b_tokens).sum::<i64>(), 40);

        assert!(model_comparison(&store, "missing", "New Display", 0, 1000)
            .unwrap()
            .is_none());
        store.hide_models(&["Old Display".into()]).unwrap();
        assert!(model_comparison(&store, "Old Display", "New Display", 0, 1000)
            .unwrap()
            .is_none());
    }

    #[test]
    fn model_comparison_rivalry_replays_daily_and_cumulative_races() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let mut events = Vec::new();
        let mut add = |id: &str, days_ago: i64, a: i64, b: i64| {
            let ts = ts_at(days_ago, 12);
            if a > 0 || days_ago == 3 {
                events.push(wrapped_event(&format!("{id}-a"), "race-a", ts, a, None, None, false));
            }
            if b > 0 || days_ago == 3 {
                events.push(wrapped_event(&format!("{id}-b"), "race-b", ts, b, None, None, false));
            }
        };
        add("d1", 8, 100, 80);
        add("d2", 7, 0, 50);
        add("d3", 6, 20, 20);
        add("d4", 5, 30, 0);
        add("d5", 4, 100, 0);
        add("zeros", 3, 0, 0);
        add("d7", 2, 10, 70);
        add("d8", 1, 0, 50);
        events.push(wrapped_event("outside", "race-a", ts_at(10, 12), 10_000, None, None, false));
        events.push(wrapped_event("other", "race-other", ts_at(1, 12), 5, None, None, false));
        store.insert_events(&events).unwrap();

        let cmp = model_comparison(&store, "race-a", "race-b", 9, 1000)
            .unwrap()
            .unwrap();
        assert_eq!((cmp.a.tokens, cmp.b.tokens), (260, 270));
        assert_eq!(cmp.total_window_tokens, 535);
        assert_eq!(cmp.daily.len(), 7, "the both-zero date is absent");
        assert!(!cmp.daily.iter().any(|d| d.date == utc_date(ts_at(3, 12))));
        assert!(cmp.daily.windows(2).all(|w| w[0].date < w[1].date));
        assert_eq!((cmp.rivalry.a_wins, cmp.rivalry.b_wins, cmp.rivalry.ties), (3, 3, 1));
        let changes: Vec<(&str, &str)> = cmp
            .rivalry
            .lead_changes
            .iter()
            .map(|c| (c.date.as_str(), c.direction.as_str()))
            .collect();
        assert_eq!(
            changes,
            [
                (utc_date(ts_at(7, 12)).as_str(), "b"),
                (utc_date(ts_at(4, 12)).as_str(), "a"),
                (utc_date(ts_at(1, 12)).as_str(), "b"),
            ]
        );
        assert_eq!(cmp.rivalry.latest_date.as_deref(), Some(utc_date(ts_at(1, 12)).as_str()));
        assert_eq!(cmp.rivalry.latest_direction.as_deref(), Some("b"));
        let gap = cmp.rivalry.biggest_gap.unwrap();
        assert_eq!(gap.date, utc_date(ts_at(4, 12)));
        assert_eq!(gap.wider_side, "a");
        assert_eq!(gap.margin, 100);
        assert_eq!(cmp.a.peak_day.as_deref(), Some(utc_date(ts_at(8, 12)).as_str()));
        assert_eq!(cmp.a.peak_day_tokens, 100, "equal peaks choose the earliest date");
    }

    #[test]
    fn model_comparison_utc_hours_night_peak_and_streaks() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let mut events = Vec::new();
        for (id, days_ago, hour, tokens) in [
            ("h1", 0, 1, 10),
            ("h2", 0, 22, 20),
            ("h3", 0, 6, 30),
            ("h4", 0, 7, 20),
            ("h5", 1, 5, 40),
            ("h6", 1, 5, 40),
            ("h7", 2, 23, 5),
        ] {
            events.push(wrapped_event(id, "utc-a", ts_at(days_ago, hour), tokens, None, None, false));
        }
        events.push(wrapped_event("utc-b", "utc-b", ts_at(0, 12), 1, None, None, false));
        store.insert_events(&events).unwrap();

        let cmp = model_comparison(&store, "utc-a", "utc-b", 3, 1000)
            .unwrap()
            .unwrap();
        let a = &cmp.a;
        assert_eq!(a.tokens, 165);
        assert_eq!((a.active_days, a.current_streak, a.longest_streak), (3, 3, 3));
        assert_eq!(a.hourly.len(), 24);
        assert!(a.hourly.iter().enumerate().all(|(hour, row)| row.hour == hour as i64));
        assert_eq!(a.hourly.iter().map(|h| h.tokens).sum::<i64>(), 165);
        assert_eq!(a.hourly[1].tokens, 10);
        assert_eq!(a.hourly[22].tokens, 20);
        assert_eq!(a.hourly[5].tokens, 80);
        assert_eq!(a.busiest_hour, Some(5));
        assert_close(a.night_share, 115.0 / 165.0);
        assert_eq!(a.peak_day.as_deref(), Some(utc_date(ts_at(1, 12)).as_str()));
        assert_eq!(a.peak_day_tokens, 80);
    }

    #[test]
    fn model_comparison_timing_uses_ratio_of_sums_and_eligibility() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        let mut events = Vec::new();
        let mut add = |id: &str,
                       output: i64,
                       duration: Option<i64>,
                       ttft: Option<i64>,
                       outcome: Option<&str>,
                       estimated: bool,
                       source: Source| {
            let mut e = wrapped_event(id, "timing-a", now, 0, None, None, estimated);
            e.output_tokens = output;
            e.duration_ms = duration;
            e.ttft_ms = ttft;
            e.outcome = outcome.map(str::to_string);
            e.source = source;
            events.push(e);
        };
        add("t1", 100, Some(1000), Some(200), Some("completed"), false, Source::Zcode);
        add("t2", 40, Some(500), Some(100), Some("completed"), false, Source::WackCode);
        add("t3", 0, Some(2000), None, None, false, Source::Zcode);
        add("t4", 30, Some(300), Some(0), Some("other"), false, Source::Zcode);
        add("t5", 10, Some(500), Some(500), None, false, Source::Zcode);
        add("failed", 1000, Some(10_000), Some(100), Some("failed"), false, Source::Zcode);
        add("cancelled", 1000, Some(10_000), Some(100), Some("cancelled"), false, Source::Zcode);
        add("estimated", 1000, Some(1000), Some(100), None, true, Source::Zcode);
        add("missing", 1000, None, Some(100), None, false, Source::Zcode);
        add("zero", 1000, Some(0), Some(0), None, false, Source::Zcode);
        events.push(wrapped_event("timing-b", "timing-b", now, 1, None, None, false));
        store.insert_events(&events).unwrap();

        let cmp = model_comparison(&store, "timing-a", "timing-b", 0, 1000)
            .unwrap()
            .unwrap();
        let timing = &cmp.a.timing;
        assert_eq!(timing.timed_calls, 5);
        assert_eq!(timing.ttft_calls, 3);
        assert_eq!(timing.generation_calls, 2, "duration == ttft is not generation time");
        assert_close(timing.avg_duration_ms.unwrap(), 860.0);
        assert_close(timing.avg_ttft_ms.unwrap(), 800.0 / 3.0);
        assert_close(timing.end_to_end_output_tps.unwrap(), 180.0 / 4.3);
        assert_close(timing.generation_output_tps.unwrap(), 140.0 / 1.2);
        assert_eq!(timing.sources.len(), 2);
        assert_eq!(timing.sources[0].source, "zcode");
        assert_eq!(timing.sources[0].timed_calls, 4);
        assert_eq!(timing.sources[1].source, "wackcode");
        assert_eq!(timing.sources[1].timed_calls, 1);
        assert_eq!(cmp.b.timing.timed_calls, 0);
        assert_eq!(cmp.b.timing.avg_duration_ms, None);
        assert_eq!(cmp.b.timing.end_to_end_output_tps, None);
        assert!(cmp.b.timing.sources.is_empty());
    }

    #[test]
    fn model_comparison_antigravity_generated_output_excludes_reasoning_from_total() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        let mut gravity = wrapped_event("generated-a", "generated-a", now, 100, None, None, false);
        gravity.source = Source::Antigravity;
        gravity.output_tokens = 20;
        gravity.reasoning_tokens = Some(30);
        gravity.cache_read_tokens = 4;
        gravity.cache_write_tokens = 5;
        gravity.duration_ms = Some(1000);
        gravity.ttft_ms = Some(200);
        let mut ordinary = wrapped_event("generated-b", "generated-b", now, 100, None, None, false);
        ordinary.output_tokens = 20;
        ordinary.reasoning_tokens = Some(30);
        ordinary.cache_read_tokens = 4;
        ordinary.cache_write_tokens = 5;
        ordinary.duration_ms = Some(1000);
        ordinary.ttft_ms = Some(200);
        store.insert_events(&[gravity, ordinary]).unwrap();

        let cmp = model_comparison(&store, "generated-a", "generated-b", 0, 1000)
            .unwrap()
            .unwrap();
        assert_eq!(cmp.a.tokens, 129);
        assert_eq!(cmp.a.output_tokens, 20);
        assert_eq!(cmp.a.reasoning_tokens, 30);
        assert_eq!(cmp.a.generated_output, 50);
        assert_close(cmp.a.timing.end_to_end_output_tps.unwrap(), 50.0);
        assert_close(cmp.a.timing.generation_output_tps.unwrap(), 62.5);
        assert_eq!(cmp.b.tokens, 129);
        assert_eq!(cmp.b.reasoning_tokens, 30);
        assert_eq!(cmp.b.generated_output, 20);
        assert_close(cmp.b.timing.end_to_end_output_tps.unwrap(), 20.0);
        assert_close(cmp.b.timing.generation_output_tps.unwrap(), 25.0);
    }

    #[test]
    fn model_comparison_empty_side_and_degenerate_inputs() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        store
            .insert_events(&[
                wrapped_event("empty-a", "gpt-5", now_ms(), 100, None, None, false),
                wrapped_event("empty-b", "gpt-4", ts_at(10, 12), 200, None, None, false),
                wrapped_event("empty-hidden", "secret", now_ms(), 50, None, None, false),
            ])
            .unwrap();
        store.hide_models(&["secret".into()]).unwrap();

        let cmp = model_comparison(&store, " gpt-5 ", "gpt-4", 5, 1000)
            .unwrap()
            .unwrap();
        assert_eq!(cmp.a.model, "gpt-5");
        assert_eq!(cmp.a.tokens, 100);
        assert_eq!(cmp.b.model, "gpt-4");
        assert_eq!(cmp.b.tokens, 0);
        assert_eq!(cmp.b.events, 0);
        assert_eq!(cmp.b.sessions, 0);
        assert_eq!(cmp.b.first_ts, None);
        assert_eq!(cmp.b.last_ts, None);
        assert_eq!(cmp.b.active_days, 0);
        assert_eq!(cmp.b.peak_day, None);
        assert_eq!(cmp.b.peak_day_tokens, 0);
        assert_eq!(cmp.b.busiest_hour, None);
        assert_eq!(cmp.b.night_share, 0.0);
        assert_eq!(cmp.b.hourly.len(), 24);
        assert!(cmp.b.hourly.iter().all(|h| h.tokens == 0));
        assert!(cmp.b.by_source.is_empty() && cmp.b.by_project.is_empty());
        assert_eq!(cmp.b.priced_cost_usd, None);
        assert_eq!(cmp.b.cost_per_mtok, None);
        assert!(cmp.cost_comparison.is_none());
        assert_eq!(cmp.total_window_tokens, 100);

        for (a, b, anchor) in [
            ("", "gpt-4", 1000),
            ("gpt-5", "  ", 1000),
            ("gpt-5", "gpt-5", 1000),
            ("gpt-5", "gpt-4", 0),
            ("gpt-5", "gpt-4", -1),
        ] {
            assert!(model_comparison(&store, a, b, 0, anchor).unwrap().is_none());
        }
        assert!(model_comparison(&store, "gpt-5", "secret", 0, 1000)
            .unwrap()
            .is_none());

        let no_race = comparison_rivalry(&[]);
        assert_eq!((no_race.a_wins, no_race.b_wins, no_race.ties), (0, 0, 0));
        assert_eq!(no_race.latest_date, None);
        assert_eq!(no_race.latest_direction, None);
        assert!(no_race.biggest_gap.is_none());
        let tied = comparison_rivalry(&[ComparisonDailyRow {
            date: "2025-01-01".into(),
            a_tokens: 10,
            b_tokens: 10,
        }]);
        assert_eq!(tied.ties, 1);
        assert!(tied.biggest_gap.is_none());
    }

    /// Events around a mid-hour alias cutover — `flash` shows as `Flash 1`
    /// (hidden) until 30 minutes into the first hour and `Flash 2` after it —
    /// in two sessions, with subagent, reasoning, estimated, priced and
    /// unpriced, NULL-model and pre-1970 rows, over two projects.
    fn rollup_fixture() -> (Store, Vec<UsageEvent>, i64, i64) {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let hour = 3_600_000;
        let h0 = 1_750_000_000_000 / hour * hour;
        let cut = h0 + 1_800_000;
        let mut events = Vec::new();
        for (i, offset) in [0, 59_000, 1_200_000, 1_800_000, 2_400_000, 3_599_999, 3_600_000, 5_000_000, 9_000_000]
            .into_iter()
            .enumerate()
        {
            for (model, session) in [("flash", Some("s1")), ("flash", Some("s2")), ("pro", None)] {
                // The cutover hour stays in one project, so its straddling
                // buckets have siblings that differ only by subagent or price.
                let project = if i >= 6 { "/repo/b" } else { "/repo/a" };
                let mut e = wrapped_event(
                    &format!("{model}-{i}-{session:?}"),
                    model,
                    h0 + offset,
                    10 + i as i64,
                    session,
                    Some(project),
                    i == 4,
                );
                e.is_subagent = session == Some("s2") && i % 2 == 1;
                e.reasoning_tokens = (i % 3 == 1).then_some(i as i64);
                e.output_tokens = 2 * i as i64;
                events.push(e);
            }
        }
        events.push(wrapped_event("old", "flash", -1_000, 7, None, None, false));
        events.push(UsageEvent { model: None, ..wrapped_event("anon", "x", h0 + 700_000, 5, Some("s1"), Some("/repo/a"), false) });
        store.insert_events(&events).unwrap();
        store
            .read_conn()
            .execute_batch(&format!(
                "INSERT INTO model_alias(alias, canonical, from_ts, until_ts) VALUES
                   ('flash', 'Flash 1', 0, {cut}), ('flash', 'Flash 2', {cut}, 9223372036854775807);
                 INSERT INTO hidden_model(name) VALUES ('Flash 1');
                 UPDATE usage_event SET cost_usd = input_tokens * 0.001 WHERE ts % 2 = 0 AND estimated = 0;"
            ))
            .unwrap();
        (store, events, h0, cut)
    }

    /// Rows as `Value`s, binding `params` by position.
    fn read_values(store: &Store, sql: &str, params: &[rusqlite::types::Value]) -> Vec<Vec<rusqlite::types::Value>> {
        let conn = store.read_conn();
        let mut st = conn.prepare(sql).unwrap();
        let n = st.column_count();
        st.query_map(rusqlite::params_from_iter(params.iter()), |r| {
            (0..n).map(|i| r.get::<_, rusqlite::types::Value>(i)).collect::<Result<Vec<_>, _>>()
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
    }

    /// `want` and `got` row for row, floats to 1e-9, ignoring the last `skip` columns.
    fn assert_same_rows(want: &[Vec<rusqlite::types::Value>], got: &[Vec<rusqlite::types::Value>], skip: usize, what: &str) {
        use rusqlite::types::Value;
        assert_eq!(want.len(), got.len(), "{what}: {want:?} vs {got:?}");
        for (w, g) in want.iter().zip(got) {
            for (a, b) in w[..w.len() - skip].iter().zip(&g[..g.len() - skip]) {
                match (a, b) {
                    (Value::Real(a), Value::Real(b)) => assert!((a - b).abs() < 1e-9, "{what}: {w:?} vs {g:?}"),
                    _ => assert_eq!(a, b, "{what}: {w:?} vs {g:?}"),
                }
            }
        }
    }

    /// Every way a query can group a row, and everything it can add up.
    fn grouped(from: &str, filter: &str) -> String {
        format!(
            "SELECT u.source, u.model, u.project, u.session_id, u.estimated, u.is_subagent,
                    u.cost_usd IS NOT NULL, date(u.ts/1000,'unixepoch'), strftime('%H', u.ts/1000,'unixepoch'),
                    COALESCE(a.canonical, u.model, 'unknown'), {NOT_HIDDEN},
                    SUM(u.n), SUM({TOKENS}), SUM(u.cost_usd), COALESCE(SUM(u.reasoning_tokens), 0),
                    SUM(CASE WHEN u.source = 'antigravity' THEN u.output_tokens + COALESCE(u.reasoning_tokens,0)
                             ELSE u.output_tokens END),
                    MIN(u.min_ts), MAX(u.max_ts), COUNT(*)
             FROM {from} u
             LEFT JOIN model_alias a ON a.alias = u.model AND u.ts >= a.from_ts AND u.ts < a.until_ts
             LEFT JOIN project_alias pj ON pj.alias = u.project
             WHERE {filter}
             GROUP BY 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11 ORDER BY 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11"
        )
    }

    /// `usage_event` as `usage_rows` columns, from `?1` on (or all of it).
    fn events_as_rows(since: Option<i64>) -> &'static str {
        match since {
            Some(_) => "(SELECT *, 1 AS n, ts AS min_ts, ts AS max_ts FROM usage_event WHERE ts >= ?1)",
            None => "(SELECT *, 1 AS n, ts AS min_ts, ts AS max_ts FROM usage_event)",
        }
    }

    /// `usage_rows` reads as the events it stands for. Grouped by everything a
    /// query groups by — display name across a mid-hour alias cutover, hidden
    /// or not, UTC day and hour, session, project, source, subagent, priced —
    /// it gives the events' own counts, sums and spans for any window start: on
    /// an hour, inside one, on the cutover, before 1970, or none.
    #[test]
    fn usage_rows_read_as_events() {
        use rusqlite::types::Value;
        let (store, events, h0, cut) = rollup_fixture();
        let hour = 3_600_000;
        let mut read_rows = 0;
        for since in [None, Some(i64::MIN), Some(0), Some(h0), Some(h0 + 1), Some(cut), Some(h0 + hour - 1), Some(h0 + hour), Some(h0 + 10 * hour)] {
            let params: Vec<Value> = since.map(Value::Integer).into_iter().collect();
            let want = read_values(&store, &grouped(events_as_rows(since), "1"), &params);
            let got = read_values(&store, &grouped(&usage_rows(since.map(|_| "?1")), "1"), &params);
            // The last column counts rows read, which is the point of the rollup.
            assert_same_rows(&want, &got, 1, &format!("since {since:?}"));
            if since.is_none() {
                read_rows = got.iter().map(|g| if let Value::Integer(n) = g[g.len() - 1] { n } else { 0 }).sum();
            }
        }
        // All time read buckets, not events — except the hour the cutover splits.
        assert!(read_rows < events.len() as i64, "{read_rows} rows for {} events", events.len());

        // And the list queries agree: `Flash 1` is hidden, the rest show.
        let models = by_model(&store, 0).unwrap();
        let names: Vec<&str> = models.iter().map(|m| m.model.as_str()).collect();
        assert!(names.contains(&"Flash 2") && names.contains(&"pro") && !names.contains(&"Flash 1"), "{names:?}");
        let flash2: i64 = events
            .iter()
            .filter(|e| e.model.as_deref() == Some("flash") && e.ts >= cut)
            .map(|e| e.input_tokens + e.output_tokens)
            .sum();
        assert_eq!(models.iter().find(|m| m.model == "Flash 2").unwrap().tokens, flash2);
    }

    /// A one-model, family or project query narrows inside `usage_rows_where`
    /// and keeps its exact test outside: it reads what the same test over the
    /// events reads, for each kind of narrowing the detail pages use.
    #[test]
    fn narrowed_rows_read_as_events() {
        use rusqlite::types::Value;
        let (store, _, h0, cut) = rollup_fixture();
        let names = vec!["Flash 2".to_string(), "pro".to_string()];
        let raw = family_raw_names(&store.read_conn(), &names).unwrap();
        let shows_family = family_shows_as(&names, 2);
        let family_narrow = family_rows(&names, &raw, 2);
        let model_narrow = model_rows(2);
        let project_narrow = |row: &str| project_rows(row, 2);
        let unknown_narrow = |row: &str| format!("({row}.project IS NULL OR {row}.project = 'unknown')");
        let shows_project = format!("{PROJECT} = ?2");
        // (what, exact test, narrowing, values bound after the window start)
        type Case<'a> = (&'a str, &'a str, &'a dyn Fn(&str) -> String, Vec<Value>);
        let cases: Vec<Case> = vec![
            ("Flash 2", SHOWS_AS_2, &model_narrow, vec![Value::Text("Flash 2".into())]),
            ("Flash 1", SHOWS_AS_2, &model_narrow, vec![Value::Text("Flash 1".into())]),
            ("unknown model", SHOWS_AS_2, &model_narrow, vec![Value::Text("unknown".into())]),
            ("family", &shows_family, &family_narrow, family_name_params(&names, &raw)),
            ("project", &shows_project, &project_narrow, vec![Value::Text("/repo/a".into())]),
            ("unknown project", "(u.project IS NULL OR u.project = 'unknown')", &unknown_narrow, vec![]),
        ];
        for (what, shows, narrow, values) in &cases {
            for since in [None, Some(h0 + 1), Some(cut), Some(h0 + 3_600_000)] {
                let mut params = vec![Value::Integer(since.unwrap_or(0))];
                params.extend(values.iter().cloned());
                // The exact test alone; the no-op `IN` only names every bound
                // parameter, which SQLite counts by the highest one used.
                let exact = format!("{shows} AND 1 IN (1, {})", holes(1, params.len()));
                let want = read_values(&store, &grouped(events_as_rows(since), &exact), &params);
                let got = read_values(
                    &store,
                    &grouped(&usage_rows_where(since.map(|_| "?1"), *narrow), &exact),
                    &params,
                );
                assert!(since.is_some() || !want.is_empty(), "{what} matched nothing");
                assert_same_rows(&want, &got, 1, &format!("{what} since {since:?}"));
            }
        }
    }

    /// The milestone walk reads hourly totals and only the crossing hour's
    /// events, yet names the same call a walk over every event would: across a
    /// mid-hour cutover (only `Flash 2`'s calls count), with several tiers
    /// crossed inside one hour and one call crossing two at once.
    #[test]
    fn milestones_name_the_crossing_call() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let hour = 3_600_000;
        let h0 = 1_750_000_000_000 / hour * hour;
        let cut = h0 + 1_800_000;
        let mut events = Vec::new();
        for i in 0..40i64 {
            let ts = h0 + i * 290_000;
            let input = match i {
                21 => 450_000_000,
                _ if ts < cut => 40_000_000,
                _ => 30_000_000,
            };
            events.push(test_event("flash", ts, input));
        }
        store.insert_events(&events).unwrap();
        store
            .read_conn()
            .execute_batch(&format!(
                "INSERT INTO model_alias(alias, canonical, from_ts, until_ts) VALUES
                   ('flash', 'Flash 1', 0, {cut}), ('flash', 'Flash 2', {cut}, 9223372036854775807);
                 UPDATE usage_event SET cost_usd = input_tokens / 1000000.0;"
            ))
            .unwrap();

        // The reference: every Flash 2 call in time order.
        let mut want: Vec<(String, String, i64)> = Vec::new();
        let (mut tokens, mut cost) = (0i64, 0.0f64);
        let token_tiers = [(100_000_000, "100m"), (500_000_000, "500m"), (1_000_000_000, "1b")];
        let cost_tiers = [(100.0, "100"), (500.0, "500"), (1000.0, "1k")];
        let (mut nt, mut nc) = (0, 0);
        let mut earned_cost = Vec::new();
        for e in events.iter().filter(|e| e.ts >= cut) {
            tokens += e.input_tokens;
            cost += e.input_tokens as f64 / 1_000_000.0;
            while nt < 3 && tokens >= token_tiers[nt].0 {
                want.push(("token_milestone".into(), token_tiers[nt].1.into(), e.ts));
                nt += 1;
            }
            while nc < 3 && cost >= cost_tiers[nc].0 {
                earned_cost.push(("big_spender".into(), cost_tiers[nc].1.into(), e.ts));
                nc += 1;
            }
        }
        want.extend(earned_cost);
        assert_eq!(want.len(), 6, "the fixture crosses every tier");

        let got: Vec<(String, String, i64)> = model_achievements(&store, "Flash 2")
            .unwrap()
            .into_iter()
            .filter(|a| a.kind == "token_milestone" || a.kind == "big_spender")
            .map(|a| (a.kind, a.tier.unwrap(), a.earned_ts.unwrap()))
            .collect();
        assert_eq!(got, want);
    }
}
