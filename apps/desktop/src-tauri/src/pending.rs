//! Names of a send that may be registered on the unit without their file
//! (`<app config dir>/pending_send.json`).
//!
//! Written before a send starts and cleared when it ends cleanly, so an unplug, a
//! "Quit Anyway" or a crash mid-send is cleaned up by `discard_unsent` on the next
//! connection instead of leaving "File missing" entries behind.

use std::path::PathBuf;

use tauri::Manager;

fn path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("pending_send.json"))
}

pub fn load(app: &tauri::AppHandle) -> Vec<String> {
    path(app)
        .ok()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save(app: &tauri::AppHandle, names: &[String]) {
    let result = path(app).and_then(|p| {
        if names.is_empty() {
            return match std::fs::remove_file(&p) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                _ => Ok(()),
            };
        }
        let tmp = p.with_extension("json.tmp");
        let body = serde_json::to_vec(names).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, body).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
    });
    if let Err(e) = result {
        log::warn!("saving the pending send list failed: {e}");
    }
}

/// `a` plus the names of `b` it doesn't have, in order.
pub fn union(a: &[String], b: &[String]) -> Vec<String> {
    let mut out = a.to_vec();
    out.extend(b.iter().filter(|n| !a.contains(n)).cloned());
    out
}

#[cfg(test)]
mod tests {
    use super::union;

    #[test]
    fn union_keeps_order_without_duplicates() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(union(&s(&["a", "b"]), &s(&["b", "c"])), s(&["a", "b", "c"]));
        assert_eq!(union(&[], &s(&["x"])), s(&["x"]));
    }
}
