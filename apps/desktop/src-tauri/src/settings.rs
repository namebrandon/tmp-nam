//! Persisted app settings (`<app config dir>/settings.json`).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::Manager;

use crate::variants;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Settings {
    /// Tone3000 publishable key (`t3k_pub_…`), used as the OAuth client id.
    #[serde(default)]
    pub t3k_key: String,
    /// Enabled variant ids (see `variants::VARIANTS`).
    #[serde(default = "variants::default_enabled")]
    pub variants: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            t3k_key: String::new(),
            variants: variants::default_enabled(),
        }
    }
}

fn path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("settings.json"))
}

pub fn load(app: &tauri::AppHandle) -> Settings {
    let mut s: Settings = path(app)
        .ok()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    s.variants = variants::migrate(s.variants);
    s
}

pub fn save(app: &tauri::AppHandle, settings: &Settings) -> Result<(), String> {
    let key = settings.t3k_key.trim();
    if !key.is_empty() && !key.starts_with("t3k_pub_") {
        return Err("The Tone3000 key should start with t3k_pub_".into());
    }
    let p = path(app)?;
    let tmp = p.with_extension("json.tmp");
    let body = serde_json::to_vec_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, body).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
}
