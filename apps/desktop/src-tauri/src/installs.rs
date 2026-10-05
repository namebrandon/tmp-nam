//! Where each capture installed from Tone3000 came from (`<app config dir>/installs.json`).
//!
//! Keyed by the SHA-256 of the installed bytes, like `/data/nam/player.json` on the
//! unit, so a rename on the unit keeps the link. Gives the UI the A1 size, the
//! "Show in Tone3000" target and each tone's "On unit" state.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::Manager;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Install {
    pub tone_id: Value,
    pub model_id: Value,
    /// Variant id (`a1-feather`…) when Tone3000 labelled the model.
    pub variant: Option<String>,
}

pub type Installs = BTreeMap<String, Install>;

fn path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("installs.json"))
}

pub fn load(app: &tauri::AppHandle) -> Installs {
    path(app)
        .ok()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn record(app: &tauri::AppHandle, entries: Vec<(String, Install)>) -> Result<(), String> {
    if entries.is_empty() {
        return Ok(());
    }
    let mut all = load(app);
    all.extend(entries);
    let p = path(app)?;
    let tmp = p.with_extension("json.tmp");
    let body = serde_json::to_vec_pretty(&all).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, body).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
}
