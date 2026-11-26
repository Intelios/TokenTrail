use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::Mutex;

use crate::store::Store;

pub struct AppState {
    /// Serializes whole-sync passes (background loop, `sync_now`) against each
    /// other and against the mutating commands. Query commands never take it:
    /// they read through `Store`'s WAL reader pool, so a running sync cannot
    /// freeze the UI.
    pub write: Mutex<()>,
    pub store: Store,
    pub home: PathBuf,
    /// Background sync passes completed. Incremented before `sync-done` is
    /// emitted, so a webview that subscribes late can tell it missed one.
    pub sync_passes: AtomicU64,
}
