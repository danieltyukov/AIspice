//! Everything the desktop backend keeps between commands.

use crate::settings::AppPrefs;
use crate::watch::ProjectWatcher;
use aispice_agent::{Config, KeyStore};
use aispice_tools::Workspace;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

pub struct AppState {
    pub ws: Arc<Workspace>,
    pub config: RwLock<Config>,
    pub prefs: RwLock<AppPrefs>,
    pub keys: KeyStore,
    /// Running agent turns, by session id, so Stop can cancel them.
    pub running: Mutex<HashMap<String, CancellationToken>>,
    /// Edits waiting for the user in ask-before-apply mode.
    pub approvals: Mutex<HashMap<String, oneshot::Sender<bool>>>,
    pub watcher: Mutex<Option<ProjectWatcher>>,
    /// The event channel of the turn currently running, used to ask for
    /// approvals.
    pub current_channel: Mutex<Option<tauri::ipc::Channel<serde_json::Value>>>,
}

impl AppState {
    pub fn new() -> Self {
        let config = Config::load().unwrap_or_default();
        let keys = KeyStore::system();
        // Keys from the old plaintext config move into the keychain on first
        // start of this version.
        let _ = keys.migrate_legacy();
        let ws = Workspace::new();
        ws.runner
            .set_config(aispice_tools::setup::runner_config(&config));
        Self {
            ws: Arc::new(ws),
            config: RwLock::new(config),
            prefs: RwLock::new(AppPrefs::load()),
            keys,
            running: Mutex::new(HashMap::new()),
            approvals: Mutex::new(HashMap::new()),
            watcher: Mutex::new(None),
            current_channel: Mutex::new(None),
        }
    }
}
