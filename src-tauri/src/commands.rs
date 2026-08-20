use crate::aggregate::{
    self, DailyCacheRow, DailyModelRow, DailyRow, FamilyStatsRow, HeatmapCell, HourRow,
    ModelDetail, ModelRow, ModelStatsRow, Overview, ProjectRow,
};
use crate::collectors;
use crate::models::{IngestStats, ModelAlias, SourceStatus};
use crate::state::AppState;
use tauri::{AppHandle, Manager, State};

#[tauri::command]
pub fn sync_now(state: State<AppState>) -> Result<Vec<IngestStats>, String> {
    let store = state.store.lock().map_err(|_| "store lock poisoned")?;
    Ok(collectors::sync_all(&store, &state.home))
}

#[tauri::command]
pub fn get_overview(state: State<AppState>) -> Result<Overview, String> {
    let store = state.store.lock().map_err(|_| "store lock poisoned")?;
    aggregate::overview(&store).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_daily(state: State<AppState>, days: i64) -> Result<Vec<DailyRow>, String> {
    let store = state.store.lock().map_err(|_| "store lock poisoned")?;
    aggregate::daily(&store, days).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_daily_by_model(state: State<AppState>, days: i64) -> Result<Vec<DailyModelRow>, String> {
    let store = state.store.lock().map_err(|_| "store lock poisoned")?;
    aggregate::daily_by_model(&store, days).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_daily_cache(state: State<AppState>, days: i64) -> Result<Vec<DailyCacheRow>, String> {
    let store = state.store.lock().map_err(|_| "store lock poisoned")?;
    aggregate::daily_cache(&store, days).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_by_model(state: State<AppState>, days: i64) -> Result<Vec<ModelRow>, String> {
    let store = state.store.lock().map_err(|_| "store lock poisoned")?;
    aggregate::by_model(&store, days).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_model_stats(state: State<AppState>, days: i64) -> Result<Vec<ModelStatsRow>, String> {
    let store = state.store.lock().map_err(|_| "store lock poisoned")?;
    aggregate::model_stats(&store, days).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_model_detail(
    state: State<AppState>,
    model: String,
) -> Result<Option<ModelDetail>, String> {
    let store = state.store.lock().map_err(|_| "store lock poisoned")?;
    aggregate::model_detail(&store, &model).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_by_project(state: State<AppState>, days: i64) -> Result<Vec<ProjectRow>, String> {
    let store = state.store.lock().map_err(|_| "store lock poisoned")?;
    aggregate::by_project(&store, days).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_heatmap(state: State<AppState>, days: i64) -> Result<Vec<HeatmapCell>, String> {
    let store = state.store.lock().map_err(|_| "store lock poisoned")?;
    aggregate::heatmap(&store, days).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_hourly(state: State<AppState>) -> Result<Vec<HourRow>, String> {
    let store = state.store.lock().map_err(|_| "store lock poisoned")?;
    aggregate::hourly(&store).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_source_status(state: State<AppState>) -> Vec<SourceStatus> {
    collectors::source_status(&state.home)
}

#[tauri::command]
pub fn get_raw_models(state: State<AppState>) -> Vec<String> {
    state
        .store
        .lock()
        .map(|store| store.get_raw_models().unwrap_or_default())
        .unwrap_or_default()
}

#[tauri::command]
pub fn get_model_aliases(state: State<AppState>) -> Vec<ModelAlias> {
    state
        .store
        .lock()
        .map(|store| store.get_model_aliases().unwrap_or_default())
        .unwrap_or_default()
}

#[tauri::command]
pub fn merge_models(state: State<AppState>, names: Vec<String>, canonical: String) -> Result<(), String> {
    let names: Vec<String> = names
        .into_iter()
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect();
    let canonical = canonical.trim().to_string();
    let store = state.store.lock().map_err(|_| "store lock poisoned")?;
    store.merge_models(&names, &canonical).map(|_| ())
}

#[tauri::command]
pub fn unmerge_models(state: State<AppState>, canonical: String) -> Result<(), String> {
    let store = state.store.lock().map_err(|_| "store lock poisoned")?;
    store
        .remove_aliases_for(&canonical)
        .map(|_| ())
        .map_err(|e| format!("unmerge models: {e}"))
}

#[tauri::command]
pub fn remove_model_alias(state: State<AppState>, alias: String) -> Result<(), String> {
    let store = state.store.lock().map_err(|_| "store lock poisoned")?;
    store
        .remove_model_alias(&alias)
        .map(|_| ())
        .map_err(|e| format!("remove model alias: {e}"))
}

#[tauri::command]
pub fn rename_model(
    state: State<AppState>,
    current_name: String,
    new_name: String,
) -> Result<(), String> {
    let store = state.store.lock().map_err(|_| "store lock poisoned")?;
    store.rename_model(&current_name, &new_name)
}

#[tauri::command]
pub fn get_hidden_models(state: State<AppState>) -> Vec<String> {
    state
        .store
        .lock()
        .map(|store| store.get_hidden_models().unwrap_or_default())
        .unwrap_or_default()
}

#[tauri::command]
pub fn hide_models(state: State<AppState>, names: Vec<String>) -> Result<(), String> {
    let names: Vec<String> = names
        .into_iter()
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect();
    let store = state.store.lock().map_err(|_| "store lock poisoned")?;
    store.hide_models(&names).map(|_| ())
}

#[tauri::command]
pub fn unhide_model(state: State<AppState>, name: String) -> Result<(), String> {
    let store = state.store.lock().map_err(|_| "store lock poisoned")?;
    store
        .unhide_model(&name)
        .map(|_| ())
        .map_err(|e| format!("unhide model: {e}"))
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
    let store = state.store.lock().map_err(|_| "store lock poisoned")?;

    let mut stmt = store
        .conn()
        .prepare(
            "SELECT source, ts, session_id, project, model, input_tokens, output_tokens,
                    reasoning_tokens, cache_read_tokens, cache_write_tokens, duration_ms, ttft_ms,
                    is_subagent, cost_usd
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
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

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
                "source,ts,session_id,project,model,input_tokens,output_tokens,reasoning_tokens,cache_read_tokens,cache_write_tokens,duration_ms,ttft_ms,is_subagent,cost_usd"
            )
            .map_err(|e| e.to_string())?;
            for r in &rows {
                writeln!(
                    out,
                    "{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
                    esc(&r.0), r.1, esc(r.2.as_deref().unwrap_or("")),
                    esc(r.3.as_deref().unwrap_or("")), esc(r.4.as_deref().unwrap_or("")),
                    r.5, r.6, r.7.unwrap_or(0), r.8, r.9,
                    r.10.unwrap_or(0), r.11.unwrap_or(0), r.12,
                    r.13.map(|c| c.to_string()).unwrap_or_default(),
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
    let store = state.store.lock().map_err(|_| "store lock poisoned")?;
    aggregate::family_stats(&store, days).map_err(|e| e.to_string())
}
