use serde::Serialize;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use time::format_description::well_known::Rfc3339;
use time::macros::format_description;

use crate::families;
use crate::store::Store;

/// "Total tokens" = input + output + both cache directions. Reasoning tokens
/// are a subset of output on both Anthropic and OpenAI and never added.
const TOKENS: &str = "(input_tokens + output_tokens + cache_read_tokens + cache_write_tokens)";

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
    pub avg_duration_ms: i64,
    pub p50_duration_ms: i64,
    pub by_purpose: Vec<UsagePurposeRow>,
    pub by_outcome: Vec<WackCodeOutcomeRow>,
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
                CAST(COALESCE(AVG(duration_ms),0) AS INTEGER),
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
            r.get::<_, i64>(11)?, r.get::<_, i64>(12)?, r.get::<_, i64>(13)?,
        ))
    })?;

    // Median duration: the OFFSET half-way point of the sorted durations. The
    // inner `u` alias shadows the outer one, so the fragment reads correctly
    // in both scopes.
    let p50_sql = format!(
        "SELECT duration_ms FROM usage_event u
         WHERE u.ts >= ?1 AND u.source='wackcode' AND duration_ms IS NOT NULL AND {H}
         ORDER BY duration_ms
         LIMIT 1 OFFSET (SELECT (COUNT(*)-1)/2 FROM usage_event u
                         WHERE u.ts >= ?1 AND u.source='wackcode' AND duration_ms IS NOT NULL AND {H})",
        H = NOT_HIDDEN
    );
    let p50_duration_ms: i64 = conn
        .query_row(&p50_sql, [cut], |r| r.get(0))
        .unwrap_or(0);

    // by_purpose runs on the connection already held here — asking the store
    // for a second one deadlocks a :memory: database (see read_conn).
    let by_purpose = {
        let mut stmt = conn.prepare(&format!(
            "SELECT COALESCE(purpose, 'chat'), COUNT(*), SUM({T}), SUM(cost_usd)
             FROM usage_event u WHERE u.ts >= ?1 AND u.source='wackcode' AND {H}
             GROUP BY purpose ORDER BY SUM({T}) DESC",
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
         FROM usage_event u LEFT JOIN model_alias a ON a.alias = u.model
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
            "SELECT date(ts/1000,'unixepoch') AS d, COALESCE(purpose,'chat'), COALESCE(SUM({T}),0)
             FROM usage_event u WHERE u.ts >= ?1 AND u.source='wackcode' AND {H}
             GROUP BY d, 2 ORDER BY d",
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
    // and models come from the row that satisfied MAX(u.ts) — the chat's most
    // recent call — so a project switched mid-chat shows the current one.
    let sessions_list = {
        let mut stmt = conn.prepare(&format!(
            "SELECT COALESCE(u.session_id, 'unknown'),
                    {P},
                    GROUP_CONCAT(DISTINCT COALESCE(a.canonical, u.model, 'unknown')),
                    COUNT(*),
                    COALESCE(SUM(u.is_subagent), 0),
                    COALESCE(SUM({T}), 0),
                    SUM(u.cost_usd),
                    COALESCE(SUM(outcome='completed'), 0),
                    COALESCE(SUM(outcome='failed'), 0),
                    COALESCE(SUM(outcome='cancelled'), 0),
                    MIN(u.ts),
                    MAX(u.ts)
             FROM usage_event u
             LEFT JOIN model_alias a ON a.alias = u.model
             LEFT JOIN project_alias pj ON pj.alias = u.project
             WHERE u.ts >= ?1 AND u.source='wackcode' AND {H}
             GROUP BY 1
             ORDER BY MAX(u.ts) DESC",
            P = PROJECT,
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
        active_days: t.13,
        cost_usd: t.7,
        first_ts: t.8,
        last_ts: t.9,
        subagent_events: t.10,
        subagent_tokens: t.11,
        avg_duration_ms: t.12,
        p50_duration_ms,
        by_purpose,
        by_outcome,
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
        "SELECT u.ts, COALESCE(u.purpose, 'chat'), u.subagent_id,
                COALESCE(a.canonical, u.model, 'unknown'),
                COALESCE(u.outcome, 'completed'),
                COALESCE({T}, 0), COALESCE(u.duration_ms, 0)
         FROM usage_event u LEFT JOIN model_alias a ON a.alias = u.model
         WHERE u.source='wackcode' AND u.session_id = ?1 AND {H}
         ORDER BY u.ts",
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
const NOT_HIDDEN: &str = "COALESCE(u.model, 'unknown') NOT IN (
    SELECT name FROM hidden_model
    UNION
    SELECT alias FROM model_alias WHERE canonical IN (SELECT name FROM hidden_model)
)";

/// The project a row counts under: its folder resolved through `project_alias`
/// (folded to its project root at ingest, or folded/kept separate by hand), with
/// NULL landing in the shared 'unknown' bucket. Every query using this needs
/// `LEFT JOIN project_alias pj ON pj.alias = u.project` in its FROM clause, and
/// must group by the evaluated expression — grouping on `u.project` instead would
/// split 'unknown' from NULL all over again.
const PROJECT: &str = "COALESCE(pj.canonical, u.project, 'unknown')";

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
    pub sessions: i64,
    pub active_days: i64,
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
    let conn = store.read_conn();
    let totals_sql = format!(
        "SELECT COALESCE(SUM({T}),0), COALESCE(SUM(input_tokens),0),
                COALESCE(SUM(output_tokens),0), COALESCE(SUM(cache_read_tokens),0),
                COALESCE(SUM(cache_write_tokens),0), COUNT(*),
                COUNT(DISTINCT session_id), SUM(cost_usd), MIN(ts), MAX(ts)
         FROM usage_event u WHERE {H}",
        T = TOKENS,
        H = NOT_HIDDEN
    );
    struct Totals {
        total: i64,
        input: i64,
        output: i64,
        cr: i64,
        cw: i64,
        events: i64,
        sessions: i64,
        cost: Option<f64>,
        first_ts: Option<i64>,
        last_ts: Option<i64>,
    }
    let t = conn.query_row(&totals_sql, [], |r| {
        Ok(Totals {
            total: r.get(0)?,
            input: r.get(1)?,
            output: r.get(2)?,
            cr: r.get(3)?,
            cw: r.get(4)?,
            events: r.get(5)?,
            sessions: r.get(6)?,
            cost: r.get(7)?,
            first_ts: r.get(8)?,
            last_ts: r.get(9)?,
        })
    })?;

    let source_sql = format!(
        "SELECT source, COALESCE(SUM({T}),0), COUNT(*), COUNT(DISTINCT session_id), SUM(cost_usd)
         FROM usage_event u WHERE {H} GROUP BY source ORDER BY 2 DESC",
        T = TOKENS,
        H = NOT_HIDDEN
    );
    let mut stmt = conn.prepare(&source_sql)?;
    let by_source = stmt
        .query_map([], |r| {
            Ok(SourceTotals {
                source: r.get(0)?,
                tokens: r.get(1)?,
                events: r.get(2)?,
                sessions: r.get(3)?,
                cost_usd: r.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut dates: Vec<String> = conn
        .prepare(&format!(
            "SELECT DISTINCT date(ts/1000, 'unixepoch') AS d FROM usage_event u WHERE {H} ORDER BY d",
            H = NOT_HIDDEN
        ))?
        .query_map([], |r| r.get(0))?
        .collect::<Result<Vec<_>, _>>()?;

    let (current, longest) = streaks(&dates);
    let active_days = dates.len() as i64;
    dates.clear();

    Ok(Overview {
        total_tokens: t.total,
        input_tokens: t.input,
        output_tokens: t.output,
        cache_read_tokens: t.cr,
        cache_write_tokens: t.cw,
        events: t.events,
        sessions: t.sessions,
        active_days,
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
         FROM usage_event u WHERE u.ts >= ?1 AND {H}",
        H = NOT_HIDDEN
    );
    store.read_conn().query_row(&sql, [cutoff(days)], |r| r.get(0))
}

pub fn daily(store: &Store, days: i64) -> DbResult<Vec<DailyRow>> {
    let sql = format!(
        "SELECT date(ts/1000,'unixepoch') AS d, source, COALESCE(SUM({T}),0), SUM(cost_usd)
         FROM usage_event u WHERE u.ts >= ?1 AND {H} GROUP BY d, source ORDER BY d",
        T = TOKENS,
        H = NOT_HIDDEN
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
         FROM usage_event u LEFT JOIN model_alias a ON a.alias = u.model
         WHERE u.ts >= ?1 AND {H} GROUP BY 1, 2 ORDER BY d",
        T = TOKENS,
        H = NOT_HIDDEN
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
         FROM usage_event u WHERE u.ts >= ?1 AND {H} GROUP BY d ORDER BY d",
        H = NOT_HIDDEN
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
        "SELECT COALESCE(a.canonical, u.model, 'unknown'), COALESCE(SUM({T}),0), COUNT(*),
                SUM(cost_usd), MAX(ts)
         FROM usage_event u LEFT JOIN model_alias a ON a.alias = u.model
         WHERE u.ts >= ?1 AND {H} GROUP BY 1 ORDER BY 2 DESC",
        T = TOKENS,
        H = NOT_HIDDEN
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
                COALESCE(SUM({T}),0), COUNT(*), COUNT(DISTINCT u.session_id),
                SUM(u.cost_usd), MIN(u.ts), MAX(u.ts), GROUP_CONCAT(DISTINCT u.source),
                COUNT(DISTINCT date(u.ts/1000, 'unixepoch'))
         FROM usage_event u LEFT JOIN model_alias a ON a.alias = u.model
         WHERE u.ts >= ?1 AND {H} GROUP BY 1 ORDER BY 6 DESC",
        T = TOKENS,
        H = NOT_HIDDEN
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
         FROM usage_event u LEFT JOIN model_alias a ON a.alias = u.model
         WHERE COALESCE(a.canonical, u.model, 'unknown') = ?1 AND {H}
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

    let sql = format!(
        "SELECT COALESCE(a.canonical, u.model, 'unknown'),
                COALESCE(SUM(u.input_tokens),0),
                COALESCE(SUM(CASE WHEN u.source = 'antigravity' THEN u.output_tokens + COALESCE(u.reasoning_tokens,0) ELSE u.output_tokens END),0),
                COALESCE(SUM(u.cache_read_tokens),0), COALESCE(SUM(u.cache_write_tokens),0),
                COALESCE(SUM(u.reasoning_tokens),0),
                COALESCE(SUM({T}),0), COUNT(*), COUNT(DISTINCT u.session_id),
                SUM(u.cost_usd), MIN(u.ts), MAX(u.ts)
         FROM usage_event u LEFT JOIN model_alias a ON a.alias = u.model
         WHERE u.ts >= ?1 AND COALESCE(a.canonical, u.model, 'unknown') = ?2 AND {H}
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
        "SELECT u.source, COALESCE(SUM({T}),0), COUNT(*), COUNT(DISTINCT u.session_id), SUM(u.cost_usd)
         FROM usage_event u LEFT JOIN model_alias a ON a.alias = u.model
         WHERE u.ts >= ?1 AND COALESCE(a.canonical, u.model, 'unknown') = ?2 AND {H}
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
        "SELECT {P}, COALESCE(SUM({T}),0), COUNT(*),
                COUNT(DISTINCT u.session_id), SUM(u.cost_usd), MIN(u.ts), MAX(u.ts)
         FROM usage_event u
         LEFT JOIN model_alias a ON a.alias = u.model
         LEFT JOIN project_alias pj ON pj.alias = u.project
         WHERE u.ts >= ?1 AND COALESCE(a.canonical, u.model, 'unknown') = ?2 AND {H}
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
         FROM usage_event u LEFT JOIN model_alias a ON a.alias = u.model
         WHERE u.ts >= ?1 AND COALESCE(a.canonical, u.model, 'unknown') = ?2 AND {H}
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

    let total_sql = format!(
        "SELECT COALESCE(SUM({T}),0) FROM usage_event u WHERE u.ts >= ?1 AND {H}",
        T = TOKENS,
        H = NOT_HIDDEN
    );
    let mut total_stmt = conn.prepare(&total_sql)?;
    let total_window_tokens: i64 = total_stmt.query_row([cutoff_ts], |r| r.get(0))?;

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

pub fn model_achievements(store: &Store, model: &str) -> DbResult<Vec<Achievement>> {
    let exists_sql = format!(
        "SELECT COALESCE(a.canonical, u.model, 'unknown')
         FROM usage_event u LEFT JOIN model_alias a ON a.alias = u.model
         WHERE COALESCE(a.canonical, u.model, 'unknown') = ?1 AND {H}
         LIMIT 1",
        H = NOT_HIDDEN
    );
    let conn = store.read_conn();
    let mut exists_stmt = conn.prepare(&exists_sql)?;
    let canonical_name = match exists_stmt.query_row(rusqlite::params![model], |r| r.get::<_, String>(0)) {
        Ok(name) => name,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };

    let mut achievements = Vec::new();

    // 1. Daily usage for Peak Day and Longest Streak
    let sql_daily = format!(
        "SELECT date(ts/1000, 'unixepoch') AS d, COALESCE(SUM({T}), 0), MIN(ts)
         FROM usage_event u LEFT JOIN model_alias a ON a.alias = u.model
         WHERE COALESCE(a.canonical, u.model, 'unknown') = ?1 AND {H}
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
        "SELECT u.source, MIN(u.ts) as first_ts
         FROM usage_event u LEFT JOIN model_alias a ON a.alias = u.model
         WHERE COALESCE(a.canonical, u.model, 'unknown') = ?1 AND {H}
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

    // 3. Family Champion
    let fam = families::family_for(&canonical_name);
    if fam != "Other" {
        let sql_models = format!(
            "SELECT COALESCE(a.canonical, u.model, 'unknown') as m, COALESCE(SUM({T}), 0) as tok
             FROM usage_event u LEFT JOIN model_alias a ON a.alias = u.model
             WHERE {H}
             GROUP BY m
             ORDER BY tok DESC",
            T = TOKENS,
            H = NOT_HIDDEN
        );
        let mut stmt_models = conn.prepare(&sql_models)?;
        let model_totals: Vec<(String, i64)> = stmt_models
            .query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let top_in_fam = model_totals
            .into_iter()
            .find(|(m, _tok)| families::family_for(m) == fam);

        if let Some((top_model, top_tokens)) = top_in_fam {
            if top_model == canonical_name && top_tokens > 0 {
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

    // 4. Token Milestones & Big Spender (scanned in chronological order)
    let sql_events = format!(
        "SELECT u.ts, {T}, COALESCE(u.cost_usd, 0.0)
         FROM usage_event u LEFT JOIN model_alias a ON a.alias = u.model
         WHERE COALESCE(a.canonical, u.model, 'unknown') = ?1 AND {H}
         ORDER BY u.ts ASC",
        T = TOKENS,
        H = NOT_HIDDEN
    );
    let mut stmt_events = conn.prepare(&sql_events)?;
    let mut event_rows = stmt_events.query(rusqlite::params![&canonical_name])?;

    let token_tiers: &[(i64, &str, &str, &str)] = &[
        (100_000_000, "100m", "100M Tokens", "100M tokens"),
        (500_000_000, "500m", "500M Tokens", "500M tokens"),
        (1_000_000_000, "1b", "1B Tokens", "1B tokens"),
    ];
    let mut next_token_tier = 0;
    let mut earned_token_milestones = Vec::new();

    let cost_tiers: &[(f64, &str, &str, &str)] = &[
        (100.0, "100", "$100 Spent", "$100 spent"),
        (500.0, "500", "$500 Spent", "$500 spent"),
        (1000.0, "1k", "$1K Spent", "$1,000 spent"),
    ];
    let mut next_cost_tier = 0;
    let mut earned_cost_milestones = Vec::new();

    let mut running_tokens: i64 = 0;
    let mut running_cost: f64 = 0.0;

    while let Some(row) = event_rows.next()? {
        let ts: i64 = row.get(0)?;
        let tokens: i64 = row.get(1)?;
        let cost: f64 = row.get(2)?;

        running_tokens += tokens;
        running_cost += cost;

        while next_token_tier < token_tiers.len() && running_tokens >= token_tiers[next_token_tier].0 {
            let (_, tier, title, val) = token_tiers[next_token_tier];
            earned_token_milestones.push(Achievement {
                kind: "token_milestone".to_string(),
                tier: Some(tier.to_string()),
                title: title.to_string(),
                value: val.to_string(),
                earned_ts: Some(ts),
            });
            next_token_tier += 1;
        }

        while next_cost_tier < cost_tiers.len() && running_cost >= cost_tiers[next_cost_tier].0 {
            let (_, tier, title, val) = cost_tiers[next_cost_tier];
            earned_cost_milestones.push(Achievement {
                kind: "big_spender".to_string(),
                tier: Some(tier.to_string()),
                title: title.to_string(),
                value: val.to_string(),
                earned_ts: Some(ts),
            });
            next_cost_tier += 1;
        }

        if next_token_tier >= token_tiers.len() && next_cost_tier >= cost_tiers.len() {
            break;
        }
    }

    achievements.extend(earned_token_milestones);
    achievements.extend(earned_cost_milestones);

    // 5. First Project & Project Explorer
    let sql_projects = format!(
        "SELECT {P} as p, MIN(u.ts) as first_ts
         FROM usage_event u
         LEFT JOIN model_alias a ON a.alias = u.model
         LEFT JOIN project_alias pj ON pj.alias = u.project
         WHERE COALESCE(a.canonical, u.model, 'unknown') = ?1 AND {H}
           AND u.project IS NOT NULL AND u.project != '' AND u.project != 'unknown'
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

pub fn by_project(store: &Store, days: i64) -> DbResult<Vec<ProjectRow>> {
    let sql = format!(
        "SELECT {P}, COALESCE(SUM({T}),0), COUNT(*),
                COUNT(DISTINCT session_id), SUM(cost_usd), MIN(ts), MAX(ts)
         FROM usage_event u
         LEFT JOIN project_alias pj ON pj.alias = u.project
         WHERE u.ts >= ?1 AND {H} GROUP BY 1 ORDER BY 2 DESC",
        T = TOKENS,
        H = NOT_HIDDEN,
        P = PROJECT
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

pub fn daily_by_project(store: &Store, days: i64) -> DbResult<Vec<DailyProjectRow>> {
    let sql = format!(
        "SELECT date(ts/1000,'unixepoch') AS d,
                {P},
                COALESCE(SUM({T}),0),
                SUM(cost_usd)
         FROM usage_event u
         LEFT JOIN project_alias pj ON pj.alias = u.project
         WHERE u.ts >= ?1 AND {H}
           AND {P} != '' AND {P} != 'unknown'
         GROUP BY d, {P}
         ORDER BY d, {P}",
        T = TOKENS,
        H = NOT_HIDDEN,
        P = PROJECT
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
             WHERE {P} = ?1 AND {H} LIMIT 1",
            H = NOT_HIDDEN,
            P = PROJECT
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
    let proj_filter = if is_unknown {
        "(u.project IS NULL OR u.project = 'unknown')".to_string()
    } else {
        format!("{P} = ?2", P = PROJECT)
    };

    let sql = format!(
        "SELECT COALESCE(SUM(u.input_tokens),0),
                COALESCE(SUM(CASE WHEN u.source = 'antigravity' THEN u.output_tokens + COALESCE(u.reasoning_tokens,0) ELSE u.output_tokens END),0),
                COALESCE(SUM(u.cache_read_tokens),0), COALESCE(SUM(u.cache_write_tokens),0),
                COALESCE(SUM(u.reasoning_tokens),0),
                COALESCE(SUM({T}),0), COUNT(*), COUNT(DISTINCT u.session_id),
                SUM(u.cost_usd), MIN(u.ts), MAX(u.ts)
         FROM usage_event u
         LEFT JOIN project_alias pj ON pj.alias = u.project
         WHERE u.ts >= ?1 AND {proj_filter} AND {H}",
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
                COUNT(*),
                COUNT(DISTINCT u.session_id),
                SUM(u.cost_usd),
                MIN(u.ts),
                MAX(u.ts)
         FROM usage_event u
         LEFT JOIN model_alias a ON a.alias = u.model
         LEFT JOIN project_alias pj ON pj.alias = u.project
         WHERE u.ts >= ?1 AND {proj_filter} AND {H}
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
        "SELECT u.source, COALESCE(SUM({T}),0), COUNT(*), COUNT(DISTINCT u.session_id), SUM(u.cost_usd)
         FROM usage_event u
         LEFT JOIN project_alias pj ON pj.alias = u.project
         WHERE u.ts >= ?1 AND {proj_filter} AND {H}
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
         FROM usage_event u
         LEFT JOIN project_alias pj ON pj.alias = u.project
         WHERE u.ts >= ?1 AND {proj_filter} AND {H}
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
                COUNT(*),
                COALESCE(SUM({T}),0),
                SUM(u.cost_usd),
                MIN(u.ts),
                MAX(u.ts),
                GROUP_CONCAT(DISTINCT COALESCE(a.canonical, u.model, 'unknown'))
         FROM usage_event u
         LEFT JOIN model_alias a ON a.alias = u.model
         LEFT JOIN project_alias pj ON pj.alias = u.project
         WHERE u.ts >= ?1 AND {proj_filter} AND {H}
         GROUP BY COALESCE(u.session_id, 'unknown'), u.source
         ORDER BY MAX(u.ts) DESC",
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
    let total_sql = format!(
        "SELECT COALESCE(SUM({T}),0) FROM usage_event u WHERE u.ts >= ?1 AND {H}",
        T = TOKENS,
        H = NOT_HIDDEN
    );
    let mut total_stmt = conn.prepare(&total_sql)?;
    let total_window_tokens: i64 = total_stmt.query_row([cutoff_ts], |r| r.get(0))?;

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
         FROM usage_event u WHERE u.ts >= ?1 AND {H} GROUP BY d ORDER BY d",
        T = TOKENS,
        H = NOT_HIDDEN
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
         FROM usage_event u WHERE {H} GROUP BY 1 ORDER BY 1",
        T = TOKENS,
        H = NOT_HIDDEN
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
         FROM usage_event u
         LEFT JOIN model_alias a ON a.alias = u.model
         WHERE u.ts >= ?1 AND {H}
         GROUP BY 1, 2
         HAVING SUM({T}) > 0
         ORDER BY tokens DESC, d DESC
         LIMIT 5",
        T = TOKENS,
        H = NOT_HIDDEN
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
    let w = format!("u.ts >= ?1 AND {H}", H = NOT_HIDDEN);
    let conn = store.read_conn();

    // One row of totals, always present (aggregates without GROUP BY never skip).
    let (total_tokens, events, sessions, first_ts, last_ts, estimated_tokens) = conn
        .query_row(
            &format!(
                "SELECT COALESCE(SUM({T}),0), COUNT(*), COUNT(DISTINCT u.session_id),
                        MIN(u.ts), MAX(u.ts),
                        COALESCE(SUM(CASE WHEN u.estimated = 1 THEN {T} ELSE 0 END),0)
                 FROM usage_event u WHERE {w}",
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
            "SELECT COALESCE(a.canonical, u.model, 'unknown'), COALESCE(SUM({T}),0), COUNT(*)
             FROM usage_event u LEFT JOIN model_alias a ON a.alias = u.model
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
             FROM usage_event u WHERE {w} GROUP BY 1 ORDER BY 2 DESC",
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
             FROM usage_event u WHERE {w} GROUP BY d ORDER BY d",
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
             FROM usage_event u WHERE {w} GROUP BY 1 ORDER BY 1",
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
             FROM usage_event u LEFT JOIN project_alias pj ON pj.alias = u.project
             WHERE {w} GROUP BY 1 ORDER BY 2 DESC LIMIT 1",
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
        "SELECT source, COUNT(*), COALESCE(SUM(estimated),0)
         FROM usage_event u WHERE {H} GROUP BY source HAVING SUM(estimated) > 0",
        H = NOT_HIDDEN
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

pub fn leaderboard_events(store: &Store, days: i64) -> DbResult<Vec<LeaderboardEvent>> {
    let cutoff_ts = cutoff(days);
    let cutoff_date: String = store.read_conn().query_row(
        "SELECT date(?1/1000, 'unixepoch')",
        [cutoff_ts],
        |r| r.get(0),
    )?;
    let is_all_time = days >= 3650 || days <= 0;

    let sql = format!(
        "SELECT date(ts/1000,'unixepoch') AS d,
                COALESCE(a.canonical, u.model, 'unknown') AS model,
                COALESCE(SUM({T}),0) AS tokens
         FROM usage_event u
         LEFT JOIN model_alias a ON a.alias = u.model
         WHERE {H}
         GROUP BY 1, 2
         ORDER BY d ASC",
        T = TOKENS,
        H = NOT_HIDDEN
    );
    let conn = store.read_conn();
    let mut stmt = conn.prepare(&sql)?;
    let rows: Vec<(String, String, i64)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<Vec<_>, _>>()?;

    let mut aliased_models: HashSet<String> = HashSet::new();
    if let Ok(mut alias_stmt) = conn.prepare("SELECT alias FROM model_alias UNION SELECT canonical FROM model_alias") {
        if let Ok(alias_rows) = alias_stmt.query_map([], |r| r.get::<_, String>(0)) {
            for name in alias_rows.flatten() {
                aliased_models.insert(name);
            }
        }
    }

    // Dense model ids in lexicographic order: id order is exactly the
    // standings' (tokens desc, name asc) tie-break, and every hot structure
    // below is a Vec indexed by id rather than a HashMap keyed by name.
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
    let n_models = names.len();

    let mut days_map: BTreeMap<String, Vec<(usize, i64)>> = BTreeMap::new();
    for (d, model, tokens) in rows {
        let id = id_of[model.as_str()];
        days_map.entry(d).or_default().push((id, tokens));
    }
    // Same-day events are emitted in model-name order, deterministically.
    for day_models in days_map.values_mut() {
        day_models.sort_by_key(|(id, _)| *id);
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
    let mut rank_since: Vec<Option<(usize, String)>> = vec![None; n_models];
    // Yesterday's ranks, materialized once at the window boundary: the
    // standings after the last pre-window day, before the first window day's
    // tokens land. Stays None until then, which is what keeps day-1 overtakes
    // (there is nothing to have been overtaken from) off.
    let mut prev_rank: Option<Vec<usize>> = None;
    let mut prev_top6: Vec<usize> = Vec::new();
    let mut seen_pre_window = false;

    let mut events: Vec<LeaderboardEvent> = Vec::new();

    for (d, day_models) in &days_map {
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
                    let (demoted_rank, entry_date) =
                        rank_since[*demoted_name].clone().unwrap_or((6, d.clone()));
                    let tenure = days_between(&entry_date, d).unwrap_or(0);

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
                    let (demoted_rank, entry_date) =
                        rank_since[*demoted_name].clone().unwrap_or((6, d.clone()));
                    let tenure = days_between(&entry_date, d).unwrap_or(0);
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

        // 8. Big-6 tenure bookkeeping: demoted models leave their rank's
        //    history; today's top 6 reset their entry date when they move.
        for demoted in &demoted_models {
            rank_since[*demoted] = None;
        }

        for (idx, model) in curr_top6.iter().enumerate() {
            let rank = idx + 1;
            if let Some((r, _)) = rank_since[*model] {
                if r != rank {
                    rank_since[*model] = Some((rank, d.clone()));
                }
            } else {
                rank_since[*model] = Some((rank, d.clone()));
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
            purpose: None, outcome: None, workspace: None, subagent_id: None,
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
        // avg (1000+4000+200+300)/4 = 1375; the median lands on the lower
        // middle of {200,300,1000,4000} — element (n-1)/2 of the sorted run
        assert_eq!(d.avg_duration_ms, 1375);
        assert_eq!(d.p50_duration_ms, 300);
        let purposes: Vec<&str> = d.by_purpose.iter().map(|p| p.purpose.as_str()).collect();
        assert_eq!(purposes, vec!["chat", "subagent", "title"]);
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
        assert_eq!(s.models, vec!["claude-sonnet-4-5", "gpt-5-mini"]);
        assert_eq!(s.events, 4);
        assert_eq!(s.subagents, 1);
        assert_eq!(s.completed, 3);
        assert_eq!(s.failed, 1);
        assert_eq!(s.cancelled, 0);

        let calls = wackcode_session_calls(&store, "chat-1").unwrap();
        assert_eq!(calls.len(), 4);
        assert_eq!(calls.iter().map(|c| c.purpose.as_str()).collect::<Vec<_>>(), vec!["chat", "subagent", "title", "chat"]);

        // the window respects the days cutoff
        let d7 = wackcode_detail(&store, 7).unwrap();
        assert_eq!(d7.events, 0);
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
    fn project_folders_fold_into_one_project() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        let mk_repo = |name: &str| {
            let dir = std::env::temp_dir().join(format!("tokentrail-test-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join(".git")).unwrap();
            dir
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
}

