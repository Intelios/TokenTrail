use serde::Serialize;
use time::format_description::well_known::Rfc3339;
use time::macros::format_description;

use crate::store::Store;

/// "Total tokens" = input + output + both cache directions. Reasoning tokens
/// are a subset of output on both Anthropic and OpenAI and never added.
const TOKENS: &str = "(input_tokens + output_tokens + cache_read_tokens + cache_write_tokens)";

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

type DbResult<T> = Result<T, rusqlite::Error>;

pub fn overview(store: &Store) -> DbResult<Overview> {
    let conn = store.conn();
    let totals_sql = format!(
        "SELECT COALESCE(SUM({T}),0), COALESCE(SUM(input_tokens),0),
                COALESCE(SUM(output_tokens),0), COALESCE(SUM(cache_read_tokens),0),
                COALESCE(SUM(cache_write_tokens),0), COUNT(*),
                COUNT(DISTINCT session_id), SUM(cost_usd), MIN(ts), MAX(ts)
         FROM usage_event",
        T = TOKENS
    );
    let (total, input, output, cr, cw, events, sessions, cost, first_ts, last_ts): (
        i64, i64, i64, i64, i64, i64, i64, Option<f64>, Option<i64>, Option<i64>,
    ) = conn.query_row(&totals_sql, [], |r| {
        Ok((
            r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?,
            r.get(7)?, r.get(8)?, r.get(9)?,
        ))
    })?;

    let source_sql = format!(
        "SELECT source, COALESCE(SUM({T}),0), COUNT(*), COUNT(DISTINCT session_id), SUM(cost_usd)
         FROM usage_event GROUP BY source ORDER BY 2 DESC",
        T = TOKENS
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
        .prepare("SELECT DISTINCT date(ts/1000, 'unixepoch') AS d FROM usage_event ORDER BY d")?
        .query_map([], |r| r.get(0))?
        .collect::<Result<Vec<_>, _>>()?;

    let (current, longest) = streaks(&dates);
    let active_days = dates.len() as i64;
    dates.clear();

    Ok(Overview {
        total_tokens: total,
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: cr,
        cache_write_tokens: cw,
        events,
        sessions,
        active_days,
        cost_usd: cost,
        first_ts,
        last_ts,
        current_streak: current,
        longest_streak: longest,
        by_source,
    })
}

pub fn daily(store: &Store, days: i64) -> DbResult<Vec<DailyRow>> {
    let sql = format!(
        "SELECT date(ts/1000,'unixepoch') AS d, source, COALESCE(SUM({T}),0), SUM(cost_usd)
         FROM usage_event WHERE ts >= ?1 GROUP BY d, source ORDER BY d",
        T = TOKENS
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
         WHERE u.ts >= ?1 GROUP BY 1, 2 ORDER BY d",
        T = TOKENS
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
    let mut stmt = store.conn().prepare(
        "SELECT date(ts/1000,'unixepoch') AS d,
                COALESCE(SUM(input_tokens),0), COALESCE(SUM(cache_write_tokens),0),
                COALESCE(SUM(cache_read_tokens),0)
         FROM usage_event WHERE ts >= ?1 GROUP BY d ORDER BY d",
    )?;
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
         WHERE u.ts >= ?1 GROUP BY 1 ORDER BY 2 DESC",
        T = TOKENS
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
         WHERE u.ts >= ?1 GROUP BY 1 ORDER BY 6 DESC",
        T = TOKENS
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

pub fn by_project(store: &Store, days: i64) -> DbResult<Vec<ProjectRow>> {
    let sql = format!(
        "SELECT COALESCE(project, 'unknown'), COALESCE(SUM({T}),0), COUNT(*),
                COUNT(DISTINCT session_id), SUM(cost_usd), MIN(ts), MAX(ts)
         FROM usage_event WHERE ts >= ?1 GROUP BY project ORDER BY 2 DESC",
        T = TOKENS
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
         FROM usage_event WHERE ts >= ?1 GROUP BY d ORDER BY d",
        T = TOKENS
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
         FROM usage_event GROUP BY 1 ORDER BY 1",
        T = TOKENS
    );
    let mut stmt = store.conn().prepare(&sql)?;
    let rows: Vec<HourRow> = stmt
        .query_map([], |r| Ok(HourRow { hour: r.get(0)?, tokens: r.get(1)? }))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
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
    Some(d.format(&day).ok()?)
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
}
