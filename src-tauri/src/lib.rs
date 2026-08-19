mod aggregate;
mod collectors;
mod commands;
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
            let data_dir = app.path().app_data_dir()?;
            let store = store::Store::open(&data_dir.join("usage.db"))
                .map_err(|e| format!("open usage store: {e}"))?;
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
            commands::get_by_project,
            commands::get_heatmap,
            commands::get_hourly,
            commands::get_source_status,
            commands::export_data
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
