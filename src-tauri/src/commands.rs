use crate::aggregate::{
    self, Achievement, Big6History, Big6Timeline, DailyCacheRow, DailyModelRow, DailyProjectRow, DailyRow,
    EstimatedShare,
    FamilyDetail, FamilyStatsRow, HeatmapCell, HourRow, LeaderboardEvent, ModelDetail, ModelRow, ModelStatsRow,
    Overview, PeakDayRow, ProjectDetail, ProjectRow, ProjectSuggestion, WackCodeCallRow,
    WackCodeDetail, WrappedSummary,
};
use crate::collectors;
use crate::models::{
    IngestStats, ModelAlias, ProjectAlias, ProjectAutoLink, ProjectColor, ProjectIgnore,
    ProjectPathRow, ProjectStack, SourceStatus,
};
use crate::state::AppState;
use tauri::{AppHandle, Manager, State};

#[tauri::command]
pub fn sync_now(state: State<AppState>) -> Result<Vec<IngestStats>, String> {
    let _sync = state.write.lock().map_err(|_| "sync lock poisoned")?;
    Ok(collectors::sync_all(&state.store, &state.home))
}

#[tauri::command]
pub fn get_overview(state: State<AppState>) -> Result<Overview, String> {
    aggregate::overview(&state.store).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_wackcode_detail(state: State<AppState>, days: i64) -> Result<WackCodeDetail, String> {
    aggregate::wackcode_detail(&state.store, days).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_wackcode_session_calls(
    state: State<AppState>,
    session_id: String,
) -> Result<Vec<WackCodeCallRow>, String> {
    aggregate::wackcode_session_calls(&state.store, &session_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_leaderboard_events(
    state: State<AppState>,
    days: i64,
) -> Result<Vec<LeaderboardEvent>, String> {
    aggregate::leaderboard_events(&state.store, days).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_daily(state: State<AppState>, days: i64) -> Result<Vec<DailyRow>, String> {
    aggregate::daily(&state.store, days).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_daily_by_model(state: State<AppState>, days: i64) -> Result<Vec<DailyModelRow>, String> {
    aggregate::daily_by_model(&state.store, days).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_daily_cache(state: State<AppState>, days: i64) -> Result<Vec<DailyCacheRow>, String> {
    aggregate::daily_cache(&state.store, days).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_by_model(state: State<AppState>, days: i64) -> Result<Vec<ModelRow>, String> {
    aggregate::by_model(&state.store, days).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_active_days(state: State<AppState>, days: i64) -> Result<i64, String> {
    aggregate::active_days_for_range(&state.store, days).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_model_stats(state: State<AppState>, days: i64) -> Result<Vec<ModelStatsRow>, String> {
    aggregate::model_stats(&state.store, days).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_model_detail(
    state: State<AppState>,
    model: String,
    days: i64,
) -> Result<Option<ModelDetail>, String> {
    aggregate::model_detail(&state.store, &model, days).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_model_achievements(
    state: State<AppState>,
    model: String,
) -> Result<Vec<Achievement>, String> {
    aggregate::model_achievements(&state.store, &model).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_model_big6(state: State<AppState>, model: String) -> Result<Option<Big6History>, String> {
    aggregate::model_big6(&state.store, &model).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_big6_timeline(state: State<AppState>) -> Result<Big6Timeline, String> {
    aggregate::big6_timeline(&state.store).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_daily_by_project(
    state: State<AppState>,
    days: i64,
) -> Result<Vec<DailyProjectRow>, String> {
    aggregate::daily_by_project(&state.store, days).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_by_project(state: State<AppState>, days: i64) -> Result<Vec<ProjectRow>, String> {
    aggregate::by_project(&state.store, days).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_project_detail(
    state: State<AppState>,
    project: String,
    days: i64,
) -> Result<Option<ProjectDetail>, String> {
    aggregate::project_detail(&state.store, &project, days).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_heatmap(state: State<AppState>, days: i64) -> Result<Vec<HeatmapCell>, String> {
    aggregate::heatmap(&state.store, days).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_hourly(state: State<AppState>) -> Result<Vec<HourRow>, String> {
    aggregate::hourly(&state.store).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_source_status(state: State<AppState>) -> Vec<SourceStatus> {
    collectors::source_status(&state.home)
}

#[tauri::command]
pub fn get_estimated_share(state: State<AppState>) -> Result<Vec<EstimatedShare>, String> {
    aggregate::estimated_share(&state.store).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_raw_models(state: State<AppState>) -> Vec<String> {
    state.store.get_raw_models().unwrap_or_default()
}

#[tauri::command]
pub fn get_model_aliases(state: State<AppState>) -> Vec<ModelAlias> {
    state.store.get_model_aliases().unwrap_or_default()
}

#[tauri::command]
pub fn merge_models(state: State<AppState>, names: Vec<String>, canonical: String) -> Result<(), String> {
    let names: Vec<String> = names
        .into_iter()
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect();
    let canonical = canonical.trim().to_string();
    let _sync = state.write.lock().map_err(|_| "sync lock poisoned")?;
    state.store.merge_models(&names, &canonical).map(|_| ())
}

#[tauri::command]
pub fn unmerge_models(state: State<AppState>, canonical: String) -> Result<(), String> {
    let _sync = state.write.lock().map_err(|_| "sync lock poisoned")?;
    state
        .store
        .remove_aliases_for(&canonical)
        .map(|_| ())
        .map_err(|e| format!("unmerge models: {e}"))
}

#[tauri::command]
pub fn remove_model_alias(state: State<AppState>, alias: String) -> Result<(), String> {
    let _sync = state.write.lock().map_err(|_| "sync lock poisoned")?;
    state
        .store
        .remove_model_alias(&alias)
        .map(|_| ())
        .map_err(|e| format!("remove model alias: {e}"))
}

/// From `at_ms` onward, show raw model `alias` as `canonical`; earlier usage
/// keeps the name it had. For floating API names that move to each new release.
#[tauri::command]
pub fn split_model_alias(
    state: State<AppState>,
    alias: String,
    at_ms: i64,
    canonical: String,
) -> Result<(), String> {
    let _sync = state.write.lock().map_err(|_| "sync lock poisoned")?;
    state.store.split_model_alias(&alias, at_ms, &canonical)
}

#[tauri::command]
pub fn remove_model_alias_segment(
    state: State<AppState>,
    alias: String,
    from_ts: i64,
) -> Result<(), String> {
    let _sync = state.write.lock().map_err(|_| "sync lock poisoned")?;
    state.store.remove_model_alias_segment(&alias, from_ts)
}

#[tauri::command]
pub fn rename_model(
    state: State<AppState>,
    current_name: String,
    new_name: String,
) -> Result<(), String> {
    let _sync = state.write.lock().map_err(|_| "sync lock poisoned")?;
    state.store.rename_model(&current_name, &new_name)
}

#[tauri::command]
pub fn get_hidden_models(state: State<AppState>) -> Vec<String> {
    state.store.get_hidden_models().unwrap_or_default()
}

#[tauri::command]
pub fn hide_models(state: State<AppState>, names: Vec<String>) -> Result<(), String> {
    let names: Vec<String> = names
        .into_iter()
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect();
    let _sync = state.write.lock().map_err(|_| "sync lock poisoned")?;
    state.store.hide_models(&names).map(|_| ())
}

#[tauri::command]
pub fn unhide_model(state: State<AppState>, name: String) -> Result<(), String> {
    let _sync = state.write.lock().map_err(|_| "sync lock poisoned")?;
    state
        .store
        .unhide_model(&name)
        .map(|_| ())
        .map_err(|e| format!("unhide model: {e}"))
}

#[tauri::command]
pub fn get_project_colors(state: State<AppState>) -> Vec<ProjectColor> {
    state.store.get_project_colors().unwrap_or_default()
}

/// `color: None` clears the project's color and returns it to the auto palette.
#[tauri::command]
pub fn set_project_color(
    state: State<AppState>,
    project: String,
    color: Option<String>,
) -> Result<(), String> {
    let _sync = state.write.lock().map_err(|_| "sync lock poisoned")?;
    match color {
        Some(c) => state.store.set_project_color(&project, &c),
        None => state
            .store
            .clear_project_color(&project)
            .map(|_| ())
            .map_err(|e| format!("clear project color: {e}")),
    }
}

#[tauri::command]
pub fn get_project_stacks(state: State<AppState>) -> Vec<ProjectStack> {
    state.store.get_project_stacks().unwrap_or_default()
}

/// Replace `project`'s tech stack tags; an empty `stack` clears all tags for the project.
#[tauri::command]
pub fn set_project_stack(
    state: State<AppState>,
    project: String,
    stack: Vec<String>,
) -> Result<(), String> {
    let _sync = state.write.lock().map_err(|_| "sync lock poisoned")?;
    state.store.set_project_stack(&project, &stack)
}

#[tauri::command]
pub fn get_project_aliases(state: State<AppState>) -> Vec<ProjectAlias> {
    state.store.get_project_aliases().unwrap_or_default()
}

/// Every recorded folder with the project it counts under and whether it still
/// exists — the merge list uses the last two to flag a project whose folder moved.
#[tauri::command]
pub fn get_project_paths(state: State<AppState>) -> Vec<ProjectPathRow> {
    state.store.get_project_paths().unwrap_or_default()
}

/// Projects whose folder is gone but which look like a live project's old home.
#[tauri::command]
pub fn get_project_suggestions(state: State<AppState>) -> Result<Vec<ProjectSuggestion>, String> {
    aggregate::project_suggestions(&state.store).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_project_auto_links(
    state: State<AppState>,
    limit: i64,
) -> Vec<ProjectAutoLink> {
    state.store.get_project_auto_links(limit).unwrap_or_default()
}

/// Undo the automatic links from `through_id` upward, newest first.
#[tauri::command]
pub fn undo_project_auto_links(state: State<AppState>, through_id: i64) -> Result<usize, String> {
    let _sync = state.write.lock().map_err(|_| "sync lock poisoned")?;
    state.store.undo_project_auto_links(through_id)
}

#[tauri::command]
pub fn get_project_ignores(state: State<AppState>) -> Vec<ProjectIgnore> {
    state.store.get_project_ignores().unwrap_or_default()
}

/// Hide a folder subtree from Projects; its usage still counts, as Unknown.
#[tauri::command]
pub fn add_project_ignore(state: State<AppState>, prefix: String) -> Result<usize, String> {
    let _sync = state.write.lock().map_err(|_| "sync lock poisoned")?;
    state.store.add_project_ignore(&prefix)
}

/// Hand an ignored subtree back to automatic grouping.
#[tauri::command]
pub fn remove_project_ignore(state: State<AppState>, prefix: String) -> Result<usize, String> {
    let _sync = state.write.lock().map_err(|_| "sync lock poisoned")?;
    state.store.remove_project_ignore(&prefix)
}

/// Re-run the project resolution pass on demand (after moving folders around).
#[tauri::command]
pub fn rescan_projects(state: State<AppState>) -> Result<usize, String> {
    let _sync = state.write.lock().map_err(|_| "sync lock poisoned")?;
    state.store.refresh_project_links()
}

#[tauri::command]
pub fn merge_projects(state: State<AppState>, names: Vec<String>, canonical: String) -> Result<(), String> {
    let names: Vec<String> = names
        .into_iter()
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect();
    let canonical = canonical.trim().to_string();
    let _sync = state.write.lock().map_err(|_| "sync lock poisoned")?;
    state.store.merge_projects(&names, &canonical).map(|_| ())
}

/// Folders kept as their own projects, overriding automatic root-folding.
#[tauri::command]
pub fn unmerge_projects(state: State<AppState>, names: Vec<String>) -> Result<(), String> {
    let names: Vec<String> = names
        .into_iter()
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect();
    let _sync = state.write.lock().map_err(|_| "sync lock poisoned")?;
    state.store.unmerge_projects(&names).map(|_| ())
}

/// Folders handed back to automatic root-folding.
#[tauri::command]
pub fn regroup_projects(state: State<AppState>, names: Vec<String>) -> Result<(), String> {
    let names: Vec<String> = names
        .into_iter()
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect();
    let _sync = state.write.lock().map_err(|_| "sync lock poisoned")?;
    state.store.regroup_projects(&names).map(|_| ())
}

#[tauri::command]
pub fn export_data(
    app: AppHandle,
    state: State<AppState>,
    format: String,
) -> Result<String, String> {
    use std::io::Write;
    let dir = app
        .path()
        .data_dir()
        .map_err(|e| e.to_string())?
        .join("TokenTrail")
        .join("exports");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
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

    let path = match format.as_str() {
        "json" => dir.join(format!("tokentrail-{stamp}.json")),
        _ => dir.join(format!("tokentrail-{stamp}.csv")),
    };
    let mut out = std::io::BufWriter::new(std::fs::File::create(&path).map_err(|e| e.to_string())?);
    match format.as_str() {
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
pub fn get_family_stats(state: State<AppState>, days: i64) -> Result<Vec<FamilyStatsRow>, String> {
    aggregate::family_stats(&state.store, days).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_family_detail(
    state: State<AppState>,
    family: String,
    days: i64,
) -> Result<Option<FamilyDetail>, String> {
    aggregate::family_detail(&state.store, &family, days).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_family_achievements(
    state: State<AppState>,
    family: String,
) -> Result<Vec<Achievement>, String> {
    aggregate::family_achievements(&state.store, &family).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_wrapped_summary(state: State<AppState>, days: i64) -> Result<WrappedSummary, String> {
    aggregate::wrapped_summary(&state.store, days).map_err(|e| e.to_string())
}

/// Write a PNG the webview rendered to a path the user picked in the save
/// dialog. A pure file write — the bytes are already final, so unlike
/// `export_data` there is no store read and no lock to hold.
#[tauri::command]
pub fn export_wrapped_png(path: String, data_b64: String) -> Result<String, String> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data_b64.as_bytes())
        .map_err(|e| format!("decode png: {e}"))?;
    if bytes.len() < 8 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
        return Err("not a PNG payload".into());
    }
    std::fs::write(&path, &bytes).map_err(|e| format!("write png: {e}"))?;
    Ok(path)
}

#[tauri::command]
pub fn get_peak_days(state: State<AppState>, days: i64) -> Result<Vec<PeakDayRow>, String> {
    aggregate::peak_days(&state.store, days).map_err(|e| e.to_string())
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
