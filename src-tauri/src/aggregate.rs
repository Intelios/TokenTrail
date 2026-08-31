use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use time::format_description::well_known::Rfc3339;
use time::macros::format_description;

use crate::families;
use crate::store::Store;

/// "Total tokens" = input + output + both cache directions. Reasoning tokens
/// are a subset of output on both Anthropic and OpenAI and never added.
const TOKENS: &str = "(input_tokens + output_tokens + cache_read_tokens + cache_write_tokens)";

/// Rows whose model is user-hidden are excluded from every aggregate. A row is
/// hidden when its raw model name is hidden, or when it aliases to a hidden
/// canonical name (hiding the display name hides all merged variants).
/// COALESCE keeps NULL-model rows visible — `NULL NOT IN (...)` is NULL, which
/// would drop them.
const NOT_HIDDEN: &str = "COALESCE(u.model,'') NOT IN (
    SELECT name FROM hidden_model
    UNION
    SELECT alias FROM model_alias WHERE canonical IN (SELECT name FROM hidden_model)
)";

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn cutoff(days: i64) -> i64 {
    now_ms() - days * 86_400_000
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
}

#[derive(Debug, Serialize)]
pub struct ModelDetail {
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
    pub active_days: i64,
    pub current_streak: i64,
    pub longest_streak: i64,
    pub peak_day: Option<String>,
    pub peak_day_tokens: i64,
    pub by_source: Vec<SourceTotals>,
    pub by_project: Vec<ProjectRow>,
    pub daily: Vec<HeatmapCell>,
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
}

type DbResult<T> = Result<T, rusqlite::Error>;

pub fn overview(store: &Store) -> DbResult<Overview> {
    let conn = store.conn();
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

pub fn daily(store: &Store, days: i64) -> DbResult<Vec<DailyRow>> {
    let sql = format!(
        "SELECT date(ts/1000,'unixepoch') AS d, source, COALESCE(SUM({T}),0), SUM(cost_usd)
         FROM usage_event u WHERE u.ts >= ?1 AND {H} GROUP BY d, source ORDER BY d",
        T = TOKENS,
        H = NOT_HIDDEN
    );
    let mut stmt = store.conn().prepare(&sql)?;
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
    let mut stmt = store.conn().prepare(&sql)?;
    let rows: Vec<DailyModelRow> = stmt
        .query_map([cutoff(days)], |r| {
            Ok(DailyModelRow { date: r.get(0)?, model: r.get(1)?, tokens: r.get(2)? })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn daily_cache(store: &Store, days: i64) -> DbResult<Vec<DailyCacheRow>> {
    let mut stmt = store.conn().prepare(&format!(
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
    let mut stmt = store.conn().prepare(&sql)?;
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
                SUM(u.cost_usd), MIN(u.ts), MAX(u.ts), GROUP_CONCAT(DISTINCT u.source)
         FROM usage_event u LEFT JOIN model_alias a ON a.alias = u.model
         WHERE u.ts >= ?1 AND {H} GROUP BY 1 ORDER BY 6 DESC",
        T = TOKENS,
        H = NOT_HIDDEN
    );
    let mut stmt = store.conn().prepare(&sql)?;
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
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Full detail card for a single model, keyed by display name (all-time).
/// Returns `None` when the model has no visible usage events.
pub fn model_detail(store: &Store, model: &str) -> DbResult<Option<ModelDetail>> {
    let sql = format!(
        "SELECT COALESCE(a.canonical, u.model, 'unknown'),
                COALESCE(SUM(u.input_tokens),0), COALESCE(SUM(u.output_tokens),0),
                COALESCE(SUM(u.cache_read_tokens),0), COALESCE(SUM(u.cache_write_tokens),0),
                COALESCE(SUM({T}),0), COUNT(*), COUNT(DISTINCT u.session_id),
                SUM(u.cost_usd), MIN(u.ts), MAX(u.ts)
         FROM usage_event u LEFT JOIN model_alias a ON a.alias = u.model
         WHERE COALESCE(a.canonical, u.model, 'unknown') = ?1 AND {H}
         GROUP BY 1",
        T = TOKENS,
        H = NOT_HIDDEN
    );
    let mut stmt = store.conn().prepare(&sql)?;
    let row = stmt.query_row(rusqlite::params![model], |r| {
        Ok((
            r.get::<_, String>(0)?,
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
    });

    let (name, inp, out, cr, cw, total, events, sessions, cost, first_ts, last_ts) = match row {
        Ok(r) => r,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
        Err(e) => return Err(e),
    };

    // by source
    let sql_src = format!(
        "SELECT u.source, COALESCE(SUM({T}),0), COUNT(*), COUNT(DISTINCT u.session_id), SUM(u.cost_usd)
         FROM usage_event u LEFT JOIN model_alias a ON a.alias = u.model
         WHERE COALESCE(a.canonical, u.model, 'unknown') = ?1 AND {H}
         GROUP BY u.source ORDER BY 2 DESC",
        T = TOKENS,
        H = NOT_HIDDEN
    );
    let mut stmt_src = store.conn().prepare(&sql_src)?;
    let by_source: Vec<SourceTotals> = stmt_src
        .query_map(rusqlite::params![model], |r| {
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
        "SELECT COALESCE(u.project, 'unknown'), COALESCE(SUM({T}),0), COUNT(*),
                COUNT(DISTINCT u.session_id), SUM(u.cost_usd), MIN(u.ts), MAX(u.ts)
         FROM usage_event u LEFT JOIN model_alias a ON a.alias = u.model
         WHERE COALESCE(a.canonical, u.model, 'unknown') = ?1 AND {H}
         GROUP BY 1 ORDER BY 2 DESC",
        T = TOKENS,
        H = NOT_HIDDEN
    );
    let mut stmt_proj = store.conn().prepare(&sql_proj)?;
    let by_project: Vec<ProjectRow> = stmt_proj
        .query_map(rusqlite::params![model], |r| {
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
         WHERE COALESCE(a.canonical, u.model, 'unknown') = ?1 AND {H}
         GROUP BY d ORDER BY d",
        T = TOKENS,
        H = NOT_HIDDEN
    );
    let mut stmt_daily = store.conn().prepare(&sql_daily)?;
    let daily: Vec<HeatmapCell> = stmt_daily
        .query_map(rusqlite::params![model], |r| {
            Ok(HeatmapCell { date: r.get(0)?, tokens: r.get(1)? })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let dates: Vec<String> = daily.iter().map(|d| d.date.clone()).collect();
    let active_days = dates.len() as i64;
    let (current_streak, longest_streak) = streaks(&dates);
    let peak = daily.iter().max_by_key(|d| d.tokens);

    Ok(Some(ModelDetail {
        model: name,
        tokens: total,
        input_tokens: inp,
        output_tokens: out,
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
    }))
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
        "SELECT COALESCE(project, 'unknown'), COALESCE(SUM({T}),0), COUNT(*),
                COUNT(DISTINCT session_id), SUM(cost_usd), MIN(ts), MAX(ts)
         FROM usage_event u WHERE u.ts >= ?1 AND {H} GROUP BY 1 ORDER BY 2 DESC",
        T = TOKENS,
        H = NOT_HIDDEN
    );
    let mut stmt = store.conn().prepare(&sql)?;
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

pub fn heatmap(store: &Store, days: i64) -> DbResult<Vec<HeatmapCell>> {
    let sql = format!(
        "SELECT date(ts/1000,'unixepoch') AS d, COALESCE(SUM({T}),0)
         FROM usage_event u WHERE u.ts >= ?1 AND {H} GROUP BY d ORDER BY d",
        T = TOKENS,
        H = NOT_HIDDEN
    );
    let mut stmt = store.conn().prepare(&sql)?;
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
    let mut stmt = store.conn().prepare(&sql)?;
    let rows: Vec<HourRow> = stmt
        .query_map([], |r| Ok(HourRow { hour: r.get(0)?, tokens: r.get(1)? }))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
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
    let mut stmt = store.conn().prepare(&sql)?;
    let rows: Vec<EstimatedShare> = stmt
        .query_map([], |r| {
            Ok(EstimatedShare { source: r.get(0)?, events: r.get(1)?, estimated: r.get(2)? })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn leaderboard_events(store: &Store, days: i64) -> DbResult<Vec<LeaderboardEvent>> {
    let cutoff_ts = cutoff(days);
    let cutoff_date: String = store.conn().query_row(
        "SELECT date(?1/1000, 'unixepoch')",
        [cutoff_ts],
        |r| r.get(0),
    )?;

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
    let mut stmt = store.conn().prepare(&sql)?;
    let rows: Vec<(String, String, i64)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<Vec<_>, _>>()?;

    let mut aliased_models: HashSet<String> = HashSet::new();
    if let Ok(mut alias_stmt) = store.conn().prepare("SELECT alias FROM model_alias UNION SELECT canonical FROM model_alias") {
        if let Ok(alias_rows) = alias_stmt.query_map([], |r| r.get::<_, String>(0)) {
            for name in alias_rows.flatten() {
                aliased_models.insert(name);
            }
        }
    }

    let mut days_map: BTreeMap<String, Vec<(String, i64)>> = BTreeMap::new();
    for (d, model, tokens) in rows {
        days_map.entry(d).or_default().push((model, tokens));
    }

    let mut cumulative_tokens: HashMap<String, i64> = HashMap::new();
    let mut first_seen_date: HashMap<String, String> = HashMap::new();
    let mut ever_top5: HashSet<String> = HashSet::new();
    let mut pre_window_daily_max: HashMap<String, i64> = HashMap::new();
    let mut in_window_daily_max: HashMap<String, i64> = HashMap::new();
    let mut prev_rankings: HashMap<String, usize> = HashMap::new();

    let mut events: Vec<LeaderboardEvent> = Vec::new();

    for (d, day_models) in &days_map {
        let in_window = d >= &cutoff_date;

        // 1. First seen & Record days
        for (model, daily_tokens) in day_models {
            let is_first_time = !first_seen_date.contains_key(model);
            if is_first_time {
                first_seen_date.insert(model.clone(), d.clone());
                if in_window && !aliased_models.contains(model) {
                    events.push(LeaderboardEvent {
                        kind: "first_seen".into(),
                        model: model.clone(),
                        other_model: None,
                        rank: None,
                        date: d.clone(),
                        tokens: *daily_tokens,
                    });
                }
            }

            if !in_window {
                let cur = pre_window_daily_max.entry(model.clone()).or_insert(0);
                if *daily_tokens > *cur {
                    *cur = *daily_tokens;
                }
            } else if pre_window_daily_max.contains_key(model) {
                let cur_record = in_window_daily_max
                    .get(model)
                    .cloned()
                    .unwrap_or_else(|| pre_window_daily_max[model]);
                if *daily_tokens > cur_record {
                    in_window_daily_max.insert(model.clone(), *daily_tokens);
                    events.push(LeaderboardEvent {
                        kind: "record".into(),
                        model: model.clone(),
                        other_model: None,
                        rank: None,
                        date: d.clone(),
                        tokens: *daily_tokens,
                    });
                }
            }
        }

        // 2. Cumulative totals
        for (model, daily_tokens) in day_models {
            *cumulative_tokens.entry(model.clone()).or_insert(0) += *daily_tokens;
        }

        // 3. Current standings across all known models
        let mut standings: Vec<(String, i64)> = cumulative_tokens
            .iter()
            .map(|(m, &t)| (m.clone(), t))
            .collect();
        standings.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

        let mut curr_rankings: HashMap<String, usize> = HashMap::new();
        for (idx, (model, _)) in standings.iter().enumerate() {
            curr_rankings.insert(model.clone(), idx + 1);
        }

        // 4. Debut (entered top 5 for the first time with prior history)
        for (model, _) in day_models {
            if let Some(&rank) = curr_rankings.get(model) {
                if rank <= 5 && !ever_top5.contains(model) {
                    let first_d = first_seen_date.get(model);
                    let has_prior_history = first_d.map(|fd| fd < d).unwrap_or(false);
                    if has_prior_history && in_window {
                        events.push(LeaderboardEvent {
                            kind: "debut".into(),
                            model: model.clone(),
                            other_model: None,
                            rank: Some(rank as i64),
                            date: d.clone(),
                            tokens: *cumulative_tokens.get(model).unwrap_or(&0),
                        });
                    }
                }
            }
        }

        // Mark models in top 5 so far
        for (idx, (model, _)) in standings.iter().enumerate() {
            if idx < 5 {
                ever_top5.insert(model.clone());
            }
        }

        // 5. Overtakes (top 10 adjacent rank swaps)
        if !prev_rankings.is_empty() && in_window {
            for (model, _) in day_models {
                if let Some(&curr_rank) = curr_rankings.get(model) {
                    if curr_rank <= 10 {
                        if let Some(&prev_rank) = prev_rankings.get(model) {
                            for (other_model, &other_prev_rank) in &prev_rankings {
                                if other_model != model && other_prev_rank < prev_rank {
                                    if let Some(&other_curr_rank) = curr_rankings.get(other_model) {
                                        if curr_rank < other_curr_rank {
                                            let my_tokens = *cumulative_tokens.get(model).unwrap_or(&0);
                                            let other_tokens = *cumulative_tokens.get(other_model).unwrap_or(&0);
                                            let gap = my_tokens - other_tokens;
                                            if gap > 0 {
                                                events.push(LeaderboardEvent {
                                                    kind: "overtake".into(),
                                                    model: model.clone(),
                                                    other_model: Some(other_model.clone()),
                                                    rank: Some(curr_rank as i64),
                                                    date: d.clone(),
                                                    tokens: gap,
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        prev_rankings = curr_rankings;
    }

    events.sort_by(|a, b| b.date.cmp(&a.date));
    events.truncate(40);
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
        let d = model_detail(&store, "GLM-5.3").unwrap().unwrap();
        assert_eq!(d.tokens, 155); // (100+20) + (30+5)
        assert_eq!(d.events, 2);

        store
            .merge_models(&["GLM-5.3".into(), "glm-5.3".into()], "GLM-5.3")
            .unwrap();

        // After merging: both variants fold under canonical name.
        let d = model_detail(&store, "GLM-5.3").unwrap().unwrap();
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
    }

    #[test]
    fn model_detail_hidden_returns_none() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let now = now_ms();
        store.insert_events(&[test_event("codex-auto-review", now, 100)]).unwrap();
        store.hide_models(&["codex-auto-review".into()]).unwrap();
        assert!(model_detail(&store, "codex-auto-review").unwrap().is_none());
    }

    #[test]
    fn model_detail_unknown_model_returns_none() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        assert!(model_detail(&store, "nonexistent-model").unwrap().is_none());
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

        let detail = model_detail(&store, "gpt-5").unwrap().unwrap();
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
}
