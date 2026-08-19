mod aggregate;
mod collectors;
mod commands;
mod families;
mod models;
mod pricing;
mod state;
mod store;

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use tauri::{Emitter, Manager};

use state::AppState;

/// Full scan on launch, then incremental sync every 30 seconds. Harness
/// databases and session files are only ever opened read-only.
const SYNC_INTERVAL: Duration = Duration::from_secs(30);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let base_data_dir = app.path().data_dir()?;
            let new_data_dir = base_data_dir.join("TokenTrail");
            let old_data_dir = app.path().app_data_dir()?;
            
            if old_data_dir.exists() && !new_data_dir.exists() {
                if let Err(e) = std::fs::rename(&old_data_dir, &new_data_dir) {
                    println!("Failed to rename data dir: {}", e);
                    let _ = std::fs::create_dir_all(&new_data_dir);
                }
            } else if !new_data_dir.exists() {
                let _ = std::fs::create_dir_all(&new_data_dir);
            }
            
            let data_dir = new_data_dir;
            
            let store = store::Store::open(&data_dir.join("usage.db"))
                .map_err(|e| format!("open usage store: {e}"))?;

            // Recompute stored costs whenever the bundled pricing table changes,
            // so history reflects updated list prices after an app update.
            let fingerprint = pricing::pricing_fingerprint();
            if store.get_watermark("pricing_fingerprint") != fingerprint as i64 {
                match store.reprice_all() {
                    Ok(n) => {
                        store.set_watermark("pricing_fingerprint", fingerprint as i64);
                        println!("repriced {n} events with updated pricing");
                    }
                    Err(e) => println!("failed to reprice history: {e}"),
                }
            }

            let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
            app.manage(AppState {
                store: Mutex::new(store),
                home: home.clone(),
            });

            let handle = app.handle().clone();
            std::thread::spawn(move || loop {
                let stats = {
                    let state = handle.state::<AppState>();
                    let store = match state.store.lock() {
                        Ok(s) => s,
                        Err(_) => break,
                    };
                    collectors::sync_all(&store, &state.home)
                };
                let _ = handle.emit("sync-done", &stats);
                std::thread::sleep(SYNC_INTERVAL);
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::sync_now,
            commands::get_overview,
            commands::get_daily,
            commands::get_daily_by_model,
            commands::get_daily_cache,
            commands::get_by_model,
            commands::get_model_stats,
            commands::get_model_detail,
            commands::get_by_project,
            commands::get_heatmap,
            commands::get_hourly,
            commands::get_source_status,
            commands::get_model_aliases,
            commands::merge_models,
            commands::unmerge_models,
            commands::remove_model_alias,
            commands::rename_model,
            commands::get_hidden_models,
            commands::hide_models,
            commands::unhide_model,
            commands::export_data,
            commands::get_family_stats
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
