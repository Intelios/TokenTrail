use std::path::PathBuf;
use std::sync::Mutex;

use crate::store::Store;

pub struct AppState {
    pub store: Mutex<Store>,
    pub home: PathBuf,
}
