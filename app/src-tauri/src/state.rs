use std::collections::HashMap;
use std::sync::Mutex;

/// Path to the persistent config file.
fn config_path() -> std::path::PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    std::path::Path::new(&home)
        .join(".config")
        .join("aispice")
        .join("config.json")
}

/// Persisted configuration (stored as JSON).
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct PersistedConfig {
    #[serde(default)]
    api_keys: HashMap<String, String>,
}

pub struct AppState {
    pub working_directory: Mutex<Option<String>>,
    /// Provider name -> API key (e.g. "openai" -> "sk-...")
    pub api_keys: Mutex<HashMap<String, String>>,
    pub ltspice_path: Mutex<Option<String>>,
}

impl AppState {
    pub fn new() -> Self {
        let mut keys = HashMap::new();

        // Load from config file first
        let cfg_path = config_path();
        if let Ok(data) = std::fs::read_to_string(&cfg_path) {
            if let Ok(cfg) = serde_json::from_str::<PersistedConfig>(&data) {
                keys = cfg.api_keys;
            }
        }

        // Env vars override persisted config (backwards compat)
        if let Ok(key) = dotenvy::var("OPENROUTER_API_KEY") {
            if !key.is_empty() {
                keys.entry("openrouter".to_string()).or_insert(key);
            }
        }
        if let Ok(key) = dotenvy::var("OPENAI_API_KEY") {
            if !key.is_empty() {
                keys.entry("openai".to_string()).or_insert(key);
            }
        }
        if let Ok(key) = dotenvy::var("ANTHROPIC_API_KEY") {
            if !key.is_empty() {
                keys.entry("anthropic".to_string()).or_insert(key);
            }
        }
        if let Ok(key) = dotenvy::var("GOOGLE_API_KEY") {
            if !key.is_empty() {
                keys.entry("google".to_string()).or_insert(key);
            }
        }

        Self {
            working_directory: Mutex::new(None),
            api_keys: Mutex::new(keys),
            ltspice_path: Mutex::new(None),
        }
    }

    /// Persist the current API keys to disk.
    pub fn persist_keys(&self) -> Result<(), String> {
        let keys = self
            .api_keys
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let cfg = PersistedConfig {
            api_keys: keys.clone(),
        };
        let json =
            serde_json::to_string_pretty(&cfg).map_err(|e| format!("Serialize error: {}", e))?;
        let cfg_path = config_path();
        if let Some(parent) = cfg_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create config dir: {}", e))?;
        }
        std::fs::write(&cfg_path, json).map_err(|e| format!("Failed to write config: {}", e))?;
        Ok(())
    }
}
