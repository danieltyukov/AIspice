//! The settings the UI edits, mapped onto aispice's config file plus a few
//! app-only preferences kept beside it.

use aispice_agent::Config;
use aispice_agent::config::EditMode;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The `Settings` shape in `app/src/ipc/types.ts`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiSettings {
    pub provider: String,
    pub model: String,
    pub base_urls: BTreeMap<String, String>,
    pub simulator: String,
    pub ltspice_path: Option<String>,
    pub reload_ltspice: bool,
    pub edit_mode: String,
    pub thinking: bool,
    pub max_steps: u32,
    pub theme: String,
}

/// Preferences only the desktop app uses.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppPrefs {
    pub reload_ltspice: bool,
    pub theme: String,
    pub recent_projects: Vec<String>,
}

impl Default for AppPrefs {
    fn default() -> Self {
        Self {
            reload_ltspice: true,
            theme: "system".into(),
            recent_projects: Vec::new(),
        }
    }
}

fn prefs_path() -> Option<PathBuf> {
    Config::default_path().and_then(|p| p.parent().map(|d| d.join("app.json")))
}

impl AppPrefs {
    pub fn load() -> Self {
        prefs_path()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Some(p) = prefs_path() {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = aispice_tools::project::atomic_write(
                &p,
                &serde_json::to_vec_pretty(self).unwrap_or_default(),
            );
        }
    }

    pub fn remember_project(&mut self, root: &str) {
        self.recent_projects.retain(|r| r != root);
        self.recent_projects.insert(0, root.to_string());
        self.recent_projects.truncate(10);
    }
}

pub fn to_ui(cfg: &Config, prefs: &AppPrefs) -> UiSettings {
    UiSettings {
        provider: cfg.provider.clone(),
        model: cfg.model_for(&cfg.provider).unwrap_or_else(|| {
            aispice_agent::providers::default_model(&cfg.provider)
                .unwrap_or("")
                .to_string()
        }),
        base_urls: cfg
            .providers
            .iter()
            .filter_map(|(k, v)| v.base_url.clone().map(|u| (k.clone(), u)))
            .collect(),
        simulator: cfg.simulator.clone(),
        ltspice_path: cfg.ltspice_path.as_ref().map(|p| p.display().to_string()),
        reload_ltspice: prefs.reload_ltspice,
        edit_mode: match cfg.edit_mode {
            EditMode::Ask => "ask".into(),
            EditMode::Apply => "apply".into(),
        },
        thinking: cfg.agent.thinking,
        max_steps: cfg.agent.max_steps,
        theme: prefs.theme.clone(),
    }
}

pub fn apply_ui(cfg: &mut Config, prefs: &mut AppPrefs, s: &UiSettings) {
    cfg.provider = s.provider.clone();
    cfg.model = Some(s.model.trim().to_string()).filter(|m| !m.is_empty());
    for (id, url) in &s.base_urls {
        let url = url.trim();
        let entry = cfg.providers.entry(id.clone()).or_default();
        entry.base_url = (!url.is_empty()).then(|| url.to_string());
    }
    cfg.simulator = s.simulator.clone();
    cfg.ltspice_path = s
        .ltspice_path
        .as_ref()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .map(PathBuf::from);
    cfg.edit_mode = if s.edit_mode == "ask" {
        EditMode::Ask
    } else {
        EditMode::Apply
    };
    cfg.agent.thinking = s.thinking;
    cfg.agent.max_steps = s.max_steps.clamp(1, 200);
    prefs.reload_ltspice = s.reload_ltspice;
    prefs.theme = s.theme.clone();
}
