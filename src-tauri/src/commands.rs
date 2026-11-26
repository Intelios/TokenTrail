use crate::aggregate::{
    self, Achievement, Big6History, Big6Timeline, DailyCacheRow, DailyModelRow, DailyProjectRow, DailyRow,
    EstimatedShare,
    FamilyDetail, FamilyStatsRow, HeatmapCell, HourRow, LeaderboardEvent, ModelComparison, ModelDetail,
    ModelRow, ModelStatsRow, Overview, PeakDayRow, ProjectDetail, ProjectRow, ProjectSuggestion, WackCodeCallRow,
    WackCodeDetail, WrappedSummary,
};
use crate::collectors;
use crate::models::{
    IngestStats, ModelAlias, ProjectAlias, ProjectAutoLink, ProjectColor, ProjectIgnore,
    ProjectPathRow, ProjectStack, SourceStatus,
};
use crate::state::AppState;
use tauri::{AppHandle, Manager};

/// Run a command body on Tauri's blocking thread pool.
///
/// A plain `#[tauri::command] fn` runs on the main thread, which on macOS is the
/// thread that paints the window: while a query ran, the whole window stopped
/// repainting, scrolling and animating, and a page's parallel invokes queued
/// behind each other on that one thread. Every command that touches the store,
/// the sync lock or the filesystem goes through here instead, so the window stays
/// live and concurrent reads spread across the store's reader pool. Blocking pool
/// rather than `#[tauri::command(async)]`, which would run the body inline on one
/// of the async runtime's few worker threads.
async fn blocking<T, F>(app: AppHandle, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&AppState) -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || f(&app.state::<AppState>()))
        .await
        .map_err(|e| format!("command task failed: {e}"))?
}

#[tauri::command]
pub async fn sync_now(app: AppHandle) -> Result<Vec<IngestStats>, String> {
    blocking(app, |state| {
        let _sync = state.write.lock().map_err(|_| "sync lock poisoned")?;
        Ok(collectors::sync_all(&state.store, &state.home))
    })
    .await
}

/// Background sync passes completed so far. The layout reads it once after
/// subscribing to `sync-done`, to catch a first pass that finished before the
/// webview was listening. An atomic read, so it stays a plain main-thread command.
#[tauri::command]
pub fn get_sync_passes(state: tauri::State<AppState>) -> u64 {
    state.sync_passes.load(std::sync::atomic::Ordering::SeqCst)
}

#[tauri::command]
pub async fn get_overview(app: AppHandle) -> Result<Overview, String> {
    blocking(app, |state| aggregate::overview(&state.store).map_err(|e| e.to_string())).await
}

#[tauri::command]
pub async fn get_wackcode_detail(app: AppHandle, days: i64) -> Result<WackCodeDetail, String> {
    blocking(app, move |state| {
        aggregate::wackcode_detail(&state.store, days).map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub async fn get_wackcode_session_calls(
    app: AppHandle,
    session_id: String,
) -> Result<Vec<WackCodeCallRow>, String> {
    blocking(app, move |state| {
        aggregate::wackcode_session_calls(&state.store, &session_id).map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub async fn get_leaderboard_events(
    app: AppHandle,
    days: i64,
) -> Result<Vec<LeaderboardEvent>, String> {
    blocking(app, move |state| {
        aggregate::leaderboard_events(&state.store, days).map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub async fn get_daily(app: AppHandle, days: i64) -> Result<Vec<DailyRow>, String> {
    blocking(app, move |state| aggregate::daily(&state.store, days).map_err(|e| e.to_string())).await
}

#[tauri::command]
pub async fn get_daily_by_model(app: AppHandle, days: i64) -> Result<Vec<DailyModelRow>, String> {
    blocking(app, move |state| {
        aggregate::daily_by_model(&state.store, days).map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub async fn get_daily_cache(app: AppHandle, days: i64) -> Result<Vec<DailyCacheRow>, String> {
    blocking(app, move |state| aggregate::daily_cache(&state.store, days).map_err(|e| e.to_string()))
        .await
}

#[tauri::command]
pub async fn get_by_model(app: AppHandle, days: i64) -> Result<Vec<ModelRow>, String> {
    blocking(app, move |state| aggregate::by_model(&state.store, days).map_err(|e| e.to_string())).await
}

#[tauri::command]
pub async fn get_active_days(app: AppHandle, days: i64) -> Result<i64, String> {
    blocking(app, move |state| {
        aggregate::active_days_for_range(&state.store, days).map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub async fn get_model_stats(app: AppHandle, days: i64) -> Result<Vec<ModelStatsRow>, String> {
    blocking(app, move |state| aggregate::model_stats(&state.store, days).map_err(|e| e.to_string()))
        .await
}

#[tauri::command]
pub async fn get_model_detail(
    app: AppHandle,
    model: String,
    days: i64,
) -> Result<Option<ModelDetail>, String> {
    blocking(app, move |state| {
        aggregate::model_detail(&state.store, &model, days).map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub async fn get_model_comparison(
    app: AppHandle,
    model_a: String,
    model_b: String,
    days: i64,
    anchor_tokens: i64,
) -> Result<ModelComparison, String> {
    let model_a = model_a.trim().to_string();
    let model_b = model_b.trim().to_string();
    if model_a.is_empty() || model_b.is_empty() {
        return Err("model names cannot be empty".into());
    }
    if model_a == model_b {
        return Err("choose two distinct models".into());
    }
    if anchor_tokens <= 0 {
        return Err("anchor_tokens must be positive".into());
    }
    blocking(app, move |state| {
        aggregate::model_comparison(&state.store, &model_a, &model_b, days, anchor_tokens)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "both models must be known, visible, and distinct".into())
    })
    .await
}

#[tauri::command]
pub async fn get_model_achievements(
    app: AppHandle,
    model: String,
) -> Result<Vec<Achievement>, String> {
    blocking(app, move |state| {
        aggregate::model_achievements(&state.store, &model).map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub async fn get_model_big6(app: AppHandle, model: String) -> Result<Option<Big6History>, String> {
    blocking(app, move |state| aggregate::model_big6(&state.store, &model).map_err(|e| e.to_string()))
        .await
}

#[tauri::command]
pub async fn get_big6_timeline(app: AppHandle) -> Result<Big6Timeline, String> {
    blocking(app, |state| aggregate::big6_timeline(&state.store).map_err(|e| e.to_string())).await
}

#[tauri::command]
pub async fn get_daily_by_project(
    app: AppHandle,
    days: i64,
) -> Result<Vec<DailyProjectRow>, String> {
    blocking(app, move |state| {
        aggregate::daily_by_project(&state.store, days).map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub async fn get_by_project(app: AppHandle, days: i64) -> Result<Vec<ProjectRow>, String> {
    blocking(app, move |state| aggregate::by_project(&state.store, days).map_err(|e| e.to_string()))
        .await
}

#[tauri::command]
pub async fn get_project_detail(
    app: AppHandle,
    project: String,
    days: i64,
) -> Result<Option<ProjectDetail>, String> {
    blocking(app, move |state| {
        aggregate::project_detail(&state.store, &project, days).map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub async fn get_heatmap(app: AppHandle, days: i64) -> Result<Vec<HeatmapCell>, String> {
    blocking(app, move |state| aggregate::heatmap(&state.store, days).map_err(|e| e.to_string())).await
}

#[tauri::command]
pub async fn get_hourly(app: AppHandle) -> Result<Vec<HourRow>, String> {
    blocking(app, |state| aggregate::hourly(&state.store).map_err(|e| e.to_string())).await
}

#[tauri::command]
pub async fn get_source_status(app: AppHandle) -> Result<Vec<SourceStatus>, String> {
    blocking(app, |state| Ok(collectors::source_status(&state.home))).await
}

#[tauri::command]
pub async fn get_estimated_share(app: AppHandle) -> Result<Vec<EstimatedShare>, String> {
    blocking(app, |state| aggregate::estimated_share(&state.store).map_err(|e| e.to_string())).await
}

#[tauri::command]
pub async fn get_raw_models(app: AppHandle) -> Result<Vec<String>, String> {
    blocking(app, |state| Ok(state.store.get_raw_models().unwrap_or_default())).await
}

#[tauri::command]
pub async fn get_model_aliases(app: AppHandle) -> Result<Vec<ModelAlias>, String> {
    blocking(app, |state| Ok(state.store.get_model_aliases().unwrap_or_default())).await
}

#[tauri::command]
pub async fn merge_models(app: AppHandle, names: Vec<String>, canonical: String) -> Result<(), String> {
    let names: Vec<String> = names
        .into_iter()
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect();
    let canonical = canonical.trim().to_string();
    blocking(app, move |state| {
        let _sync = state.write.lock().map_err(|_| "sync lock poisoned")?;
        state.store.merge_models(&names, &canonical).map(|_| ())
    })
    .await
}

#[tauri::command]
pub async fn unmerge_models(app: AppHandle, canonical: String) -> Result<(), String> {
    blocking(app, move |state| {
        let _sync = state.write.lock().map_err(|_| "sync lock poisoned")?;
        state
            .store
            .remove_aliases_for(&canonical)
            .map(|_| ())
            .map_err(|e| format!("unmerge models: {e}"))
    })
    .await
}

#[tauri::command]
pub async fn remove_model_alias(app: AppHandle, alias: String) -> Result<(), String> {
    blocking(app, move |state| {
        let _sync = state.write.lock().map_err(|_| "sync lock poisoned")?;
        state
            .store
            .remove_model_alias(&alias)
            .map(|_| ())
            .map_err(|e| format!("remove model alias: {e}"))
    })
    .await
}

/// From `at_ms` onward, show raw model `alias` as `canonical`; earlier usage
/// keeps the name it had. For floating API names that move to each new release.
#[tauri::command]
pub async fn split_model_alias(
    app: AppHandle,
    alias: String,
    at_ms: i64,
    canonical: String,
) -> Result<(), String> {
    blocking(app, move |state| {
        let _sync = state.write.lock().map_err(|_| "sync lock poisoned")?;
        state.store.split_model_alias(&alias, at_ms, &canonical)
    })
    .await
}

#[tauri::command]
pub async fn remove_model_alias_segment(
    app: AppHandle,
    alias: String,
    from_ts: i64,
) -> Result<(), String> {
    blocking(app, move |state| {
        let _sync = state.write.lock().map_err(|_| "sync lock poisoned")?;
        state.store.remove_model_alias_segment(&alias, from_ts)
    })
    .await
}

#[tauri::command]
pub async fn rename_model(
    app: AppHandle,
    current_name: String,
    new_name: String,
) -> Result<(), String> {
    blocking(app, move |state| {
        let _sync = state.write.lock().map_err(|_| "sync lock poisoned")?;
        state.store.rename_model(&current_name, &new_name)
    })
    .await
}

#[tauri::command]
pub async fn get_hidden_models(app: AppHandle) -> Result<Vec<String>, String> {
    blocking(app, |state| Ok(state.store.get_hidden_models().unwrap_or_default())).await
}

#[tauri::command]
pub async fn hide_models(app: AppHandle, names: Vec<String>) -> Result<(), String> {
    let names: Vec<String> = names
        .into_iter()
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect();
    blocking(app, move |state| {
        let _sync = state.write.lock().map_err(|_| "sync lock poisoned")?;
        state.store.hide_models(&names).map(|_| ())
    })
    .await
}

#[tauri::command]
pub async fn unhide_model(app: AppHandle, name: String) -> Result<(), String> {
    blocking(app, move |state| {
        let _sync = state.write.lock().map_err(|_| "sync lock poisoned")?;
        state
            .store
            .unhide_model(&name)
            .map(|_| ())
            .map_err(|e| format!("unhide model: {e}"))
    })
    .await
}

#[tauri::command]
pub async fn get_project_colors(app: AppHandle) -> Result<Vec<ProjectColor>, String> {
    blocking(app, |state| Ok(state.store.get_project_colors().unwrap_or_default())).await
}

/// `color: None` clears the project's color and returns it to the auto palette.
#[tauri::command]
pub async fn set_project_color(
    app: AppHandle,
    project: String,
    color: Option<String>,
) -> Result<(), String> {
    blocking(app, move |state| {
        let _sync = state.write.lock().map_err(|_| "sync lock poisoned")?;
        match color {
            Some(c) => state.store.set_project_color(&project, &c),
            None => state
                .store
                .clear_project_color(&project)
                .map(|_| ())
                .map_err(|e| format!("clear project color: {e}")),
        }
    })
    .await
}

#[tauri::command]
pub async fn get_project_stacks(app: AppHandle) -> Result<Vec<ProjectStack>, String> {
    blocking(app, |state| Ok(state.store.get_project_stacks().unwrap_or_default())).await
}

/// Replace `project`'s tech stack tags; an empty `stack` clears all tags for the project.
#[tauri::command]
pub async fn set_project_stack(
    app: AppHandle,
    project: String,
    stack: Vec<String>,
) -> Result<(), String> {
    blocking(app, move |state| {
        let _sync = state.write.lock().map_err(|_| "sync lock poisoned")?;
        state.store.set_project_stack(&project, &stack)
    })
    .await
}

#[tauri::command]
pub async fn get_project_aliases(app: AppHandle) -> Result<Vec<ProjectAlias>, String> {
    blocking(app, |state| Ok(state.store.get_project_aliases().unwrap_or_default())).await
}

/// Every recorded folder with the project it counts under and whether it still
/// exists — the merge list uses the last two to flag a project whose folder moved.
#[tauri::command]
pub async fn get_project_paths(app: AppHandle) -> Result<Vec<ProjectPathRow>, String> {
    blocking(app, |state| Ok(state.store.get_project_paths().unwrap_or_default())).await
}

/// Projects whose folder is gone but which look like a live project's old home.
#[tauri::command]
pub async fn get_project_suggestions(app: AppHandle) -> Result<Vec<ProjectSuggestion>, String> {
    blocking(app, |state| aggregate::project_suggestions(&state.store).map_err(|e| e.to_string()))
        .await
}

#[tauri::command]
pub async fn get_project_auto_links(
    app: AppHandle,
    limit: i64,
) -> Result<Vec<ProjectAutoLink>, String> {
    blocking(app, move |state| Ok(state.store.get_project_auto_links(limit).unwrap_or_default()))
        .await
}

/// Undo the automatic links from `through_id` upward, newest first.
#[tauri::command]
pub async fn undo_project_auto_links(app: AppHandle, through_id: i64) -> Result<usize, String> {
    blocking(app, move |state| {
        let _sync = state.write.lock().map_err(|_| "sync lock poisoned")?;
        state.store.undo_project_auto_links(through_id)
    })
    .await
}

#[tauri::command]
pub async fn get_project_ignores(app: AppHandle) -> Result<Vec<ProjectIgnore>, String> {
    blocking(app, |state| Ok(state.store.get_project_ignores().unwrap_or_default())).await
}

/// Hide a folder subtree from Projects; its usage still counts, as Unknown.
#[tauri::command]
pub async fn add_project_ignore(app: AppHandle, prefix: String) -> Result<usize, String> {
    blocking(app, move |state| {
        let _sync = state.write.lock().map_err(|_| "sync lock poisoned")?;
        state.store.add_project_ignore(&prefix)
    })
    .await
}

/// Hand an ignored subtree back to automatic grouping.
#[tauri::command]
pub async fn remove_project_ignore(app: AppHandle, prefix: String) -> Result<usize, String> {
    blocking(app, move |state| {
        let _sync = state.write.lock().map_err(|_| "sync lock poisoned")?;
        state.store.remove_project_ignore(&prefix)
    })
    .await
}

/// Re-run the project resolution pass on demand (after moving folders around).
#[tauri::command]
pub async fn rescan_projects(app: AppHandle) -> Result<usize, String> {
    blocking(app, |state| {
        let _sync = state.write.lock().map_err(|_| "sync lock poisoned")?;
        state.store.refresh_project_links()
    })
    .await
}

#[tauri::command]
pub async fn merge_projects(app: AppHandle, names: Vec<String>, canonical: String) -> Result<(), String> {
    let names: Vec<String> = names
        .into_iter()
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect();
    let canonical = canonical.trim().to_string();
    blocking(app, move |state| {
        let _sync = state.write.lock().map_err(|_| "sync lock poisoned")?;
        state.store.merge_projects(&names, &canonical).map(|_| ())
    })
    .await
}

/// Folders kept as their own projects, overriding automatic root-folding.
#[tauri::command]
pub async fn unmerge_projects(app: AppHandle, names: Vec<String>) -> Result<(), String> {
    let names: Vec<String> = names
        .into_iter()
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect();
    blocking(app, move |state| {
        let _sync = state.write.lock().map_err(|_| "sync lock poisoned")?;
        state.store.unmerge_projects(&names).map(|_| ())
    })
    .await
}

/// Folders handed back to automatic root-folding.
#[tauri::command]
pub async fn regroup_projects(app: AppHandle, names: Vec<String>) -> Result<(), String> {
    let names: Vec<String> = names
        .into_iter()
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect();
    blocking(app, move |state| {
        let _sync = state.write.lock().map_err(|_| "sync lock poisoned")?;
        state.store.regroup_projects(&names).map(|_| ())
    })
    .await
}

#[tauri::command]
pub async fn export_data(app: AppHandle, format: String) -> Result<String, String> {
    let dir = app
        .path()
        .data_dir()
        .map_err(|e| e.to_string())?
        .join("TokenTrail")
        .join("exports");
    blocking(app, move |state| export_rows(state, &dir, &format)).await
}

/// The body of `export_data`, run on the blocking pool: a full-table read and a
/// file write that can take seconds on a large history.
fn export_rows(state: &AppState, dir: &std::path::Path, format: &str) -> Result<String, String> {
    use std::io::Write;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Held so no sync or mutation can land mid-export: the file then shows one
    // consistent view. Reads elsewhere are unaffected — they use their own
    // connections.
    let _sync = state.write.lock().map_err(|_| "sync lock poisoned")?;
    let conn = state.store.read_conn();

    let mut stmt = conn
        .prepare(
            "SELECT source, ts, session_id, project, model, input_tokens, output_tokens,
                    reasoning_tokens, cache_read_tokens, cache_write_tokens, duration_ms, ttft_ms,
                    is_subagent, cost_usd, estimated, purpose, outcome, workspace, subagent_id, mode
             FROM usage_event ORDER BY ts",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, Option<i64>>(7)?,
                r.get::<_, i64>(8)?,
                r.get::<_, i64>(9)?,
                r.get::<_, Option<i64>>(10)?,
                r.get::<_, Option<i64>>(11)?,
                r.get::<_, i64>(12)?,
                r.get::<_, Option<f64>>(13)?,
                r.get::<_, i64>(14)?,
                r.get::<_, Option<String>>(15)?,
                r.get::<_, Option<String>>(16)?,
                r.get::<_, Option<String>>(17)?,
                r.get::<_, Option<String>>(18)?,
                r.get::<_, Option<String>>(19)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(stmt);

    let path = match format {
        "json" => dir.join(format!("tokentrail-{stamp}.json")),
        _ => dir.join(format!("tokentrail-{stamp}.csv")),
    };
    let mut out = std::io::BufWriter::new(std::fs::File::create(&path).map_err(|e| e.to_string())?);
    match format {
        "json" => {
            let items: Vec<serde_json::Value> = rows
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "source": r.0, "ts": r.1, "session_id": r.2, "project": r.3,
                        "model": r.4, "input_tokens": r.5, "output_tokens": r.6,
                        "reasoning_tokens": r.7, "cache_read_tokens": r.8,
                        "cache_write_tokens": r.9, "duration_ms": r.10, "ttft_ms": r.11,
                        "is_subagent": r.12 != 0, "cost_usd": r.13,
                        "estimated": r.14 != 0,
                        "purpose": r.15, "outcome": r.16, "workspace": r.17, "subagent_id": r.18,
                        "mode": r.19,
                    })
                })
                .collect();
            serde_json::to_writer_pretty(&mut out, &items).map_err(|e| e.to_string())?;
        }
        _ => {
            let esc = |s: &str| {
                if s.contains(',') || s.contains('"') || s.contains('\n') {
                    format!("\"{}\"", s.replace('"', "\"\""))
                } else {
                    s.to_string()
                }
            };
            writeln!(
                out,
                "source,ts,session_id,project,model,input_tokens,output_tokens,reasoning_tokens,cache_read_tokens,cache_write_tokens,duration_ms,ttft_ms,is_subagent,cost_usd,estimated,purpose,outcome,workspace,subagent_id,mode"
            )
            .map_err(|e| e.to_string())?;
            for r in &rows {
                writeln!(
                    out,
                    "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
                    esc(&r.0), r.1, esc(r.2.as_deref().unwrap_or("")),
                    esc(r.3.as_deref().unwrap_or("")), esc(r.4.as_deref().unwrap_or("")),
                    r.5, r.6, r.7.unwrap_or(0), r.8, r.9,
                    r.10.map(|d| d.to_string()).unwrap_or_default(),
                    r.11.map(|t| t.to_string()).unwrap_or_default(),
                    r.12,
                    r.13.map(|c| c.to_string()).unwrap_or_default(),
                    r.14,
                    esc(r.15.as_deref().unwrap_or("")), esc(r.16.as_deref().unwrap_or("")),
                    esc(r.17.as_deref().unwrap_or("")), esc(r.18.as_deref().unwrap_or("")),
                    esc(r.19.as_deref().unwrap_or("")),
                )
                .map_err(|e| e.to_string())?;
            }
        }
    }
    out.flush().map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

#[tauri::command]
pub async fn get_family_stats(app: AppHandle, days: i64) -> Result<Vec<FamilyStatsRow>, String> {
    blocking(app, move |state| aggregate::family_stats(&state.store, days).map_err(|e| e.to_string()))
        .await
}

#[tauri::command]
pub async fn get_family_detail(
    app: AppHandle,
    family: String,
    days: i64,
) -> Result<Option<FamilyDetail>, String> {
    blocking(app, move |state| {
        aggregate::family_detail(&state.store, &family, days).map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub async fn get_family_achievements(
    app: AppHandle,
    family: String,
) -> Result<Vec<Achievement>, String> {
    blocking(app, move |state| {
        aggregate::family_achievements(&state.store, &family).map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub async fn get_wrapped_summary(app: AppHandle, days: i64) -> Result<WrappedSummary, String> {
    blocking(app, move |state| {
        aggregate::wrapped_summary(&state.store, days).map_err(|e| e.to_string())
    })
    .await
}

/// Write a PNG the webview rendered to a path the user picked in the save
/// dialog. A pure file write — the bytes are already final, so unlike
/// `export_data` there is no store read and no lock to hold.
#[tauri::command]
pub async fn export_wrapped_png(app: AppHandle, path: String, data_b64: String) -> Result<String, String> {
    blocking(app, move |_| {
        use base64::Engine as _;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data_b64.as_bytes())
            .map_err(|e| format!("decode png: {e}"))?;
        if bytes.len() < 8 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
            return Err("not a PNG payload".into());
        }
        std::fs::write(&path, &bytes).map_err(|e| format!("write png: {e}"))?;
        Ok(path)
    })
    .await
}

#[tauri::command]
pub async fn get_peak_days(app: AppHandle, days: i64) -> Result<Vec<PeakDayRow>, String> {
    blocking(app, move |state| aggregate::peak_days(&state.store, days).map_err(|e| e.to_string()))
        .await
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_csv_export_handles_missing_latencies() {
        let r = (
            "antigravity".to_string(),
            1700000000000_i64,
            Some("session-1".to_string()),
            Some("proj".to_string()),
            Some("gemini-1.5-pro".to_string()),
            100_i64,
            50_i64,
            None::<i64>,
            0_i64,
            0_i64,
            None::<i64>, // duration_ms missing
            None::<i64>, // ttft_ms missing
            0_i64,
            Some(0.001_f64),
        );

        let esc = |s: &str| {
            if s.contains(',') || s.contains('"') || s.contains('\n') {
                format!("\"{}\"", s.replace('"', "\"\""))
            } else {
                s.to_string()
            }
        };

        let formatted = format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            esc(&r.0),
            r.1,
            esc(r.2.as_deref().unwrap_or("")),
            esc(r.3.as_deref().unwrap_or("")),
            esc(r.4.as_deref().unwrap_or("")),
            r.5,
            r.6,
            r.7.unwrap_or(0),
            r.8,
            r.9,
            r.10.map(|d| d.to_string()).unwrap_or_default(),
            r.11.map(|t| t.to_string()).unwrap_or_default(),
            r.12,
            r.13.map(|c| c.to_string()).unwrap_or_default(),
        );

        assert_eq!(
            formatted,
            "antigravity,1700000000000,session-1,proj,gemini-1.5-pro,100,50,0,0,0,,,0,0.001"
        );
    }

    #[test]
    fn test_csv_export_includes_present_latencies() {
        let r = (
            "antigravity".to_string(),
            1700000000000_i64,
            Some("session-1".to_string()),
            Some("proj".to_string()),
            Some("gemini-1.5-pro".to_string()),
            100_i64,
            50_i64,
            None::<i64>,
            0_i64,
            0_i64,
            Some(1250_i64), // duration_ms present
            Some(320_i64),  // ttft_ms present
            0_i64,
            Some(0.001_f64),
        );

        let esc = |s: &str| {
            if s.contains(',') || s.contains('"') || s.contains('\n') {
                format!("\"{}\"", s.replace('"', "\"\""))
            } else {
                s.to_string()
            }
        };

        let formatted = format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            esc(&r.0),
            r.1,
            esc(r.2.as_deref().unwrap_or("")),
            esc(r.3.as_deref().unwrap_or("")),
            esc(r.4.as_deref().unwrap_or("")),
            r.5,
            r.6,
            r.7.unwrap_or(0),
            r.8,
            r.9,
            r.10.map(|d| d.to_string()).unwrap_or_default(),
            r.11.map(|t| t.to_string()).unwrap_or_default(),
            r.12,
            r.13.map(|c| c.to_string()).unwrap_or_default(),
        );

        assert_eq!(
            formatted,
            "antigravity,1700000000000,session-1,proj,gemini-1.5-pro,100,50,0,0,0,1250,320,0,0.001"
        );
    }
}

#[tauri::command]
pub async fn get_wackchatter_libraries(
    app: AppHandle,
) -> Result<Vec<crate::wackchatter::Library>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let s = app.state::<AppState>();
        crate::wackchatter::libraries(&s.store, &s.home)
    })
    .await
    .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn get_wackchatter_stats(
    app: AppHandle,
    library_id: String,
    character_id: Option<String>,
    group_id: Option<String>,
) -> Result<crate::wackchatter::Stats, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let s = app.state::<AppState>();
        crate::wackchatter::stats(
            &s.store,
            &s.home,
            &library_id,
            character_id.as_deref(),
            group_id.as_deref(),
        )
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn get_wackchatter_usage(
    app: AppHandle,
    library_id: Option<String>,
    days: i64,
    session_id: Option<String>,
) -> Result<crate::wackchatter::Usage, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let s = app.state::<AppState>();
        crate::wackchatter::usage(&s.store, library_id.as_deref(), days, session_id.as_deref())
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn get_wackchatter_portrait(
    app: AppHandle,
    library_id: String,
    id: String,
    persona: bool,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let s = app.state::<AppState>();
        crate::wackchatter::portrait(&s.store, &s.home, &library_id, &id, persona)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn open_wackchatter(
    app: AppHandle,
    library_id: String,
    chat_id: Option<String>,
    character_id: Option<String>,
) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let home = app.state::<AppState>().home.clone();
    let url = crate::wackchatter::destination(
        &home,
        &library_id,
        chat_id.as_deref(),
        character_id.as_deref(),
    )
    .await?;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
}
