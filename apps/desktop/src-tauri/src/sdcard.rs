//! Bootable SD card, backed by the `tmp-sdcard` crate.
//!
//! The card is written by a child process — this same executable in
//! `--sdcard-helper write` mode — run as root through `osascript … with
//! administrator privileges` (macOS) or `pkexec` (Linux). osascript doesn't
//! stream, so the helper writes to a log file that is tailed. (Portable image files
//! are a `tmp-sdcard image` CLI feature; the app only writes cards.)
//!
//! The helper prints one JSON object per line (`tmp_sdcard::util::JsonReporter`).

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::{Emitter, Manager};
use tmp_sdcard::{card, DeviceDir, Release};

#[derive(Serialize, Clone)]
pub struct Tool {
    pub name: String,
    pub path: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct Environment {
    pub kit_root: Option<String>,
    pub kit_error: Option<String>,
    pub tools: Vec<Tool>,
    pub default_firmware: Option<String>,
    pub firmware_sha256: String,
    pub platform: String,
}

#[derive(Serialize, Clone)]
pub struct Firmware {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
    pub matches: bool,
}

#[derive(Serialize, Clone)]
pub struct LogLine {
    pub line: String,
    pub percent: Option<f64>,
    pub label: Option<String>,
    /// Index into the UI's eight build stages (see `stage`).
    pub stage: Option<usize>,
    /// Progress within the stage, 0–1.
    pub fraction: Option<f64>,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Ok,
    Failed,
    /// The administrator prompt was cancelled; nothing was written.
    Denied,
}

#[derive(Serialize, Clone)]
pub struct Finished {
    pub ok: bool,
    pub code: i32,
    pub message: String,
    pub outcome: Outcome,
    /// The stage that was running when the build failed.
    pub stage: Option<usize>,
}

/// The builder's progress labels, in order, as the UI's eight stages: check the
/// firmware, extract, verify, add NAM support, build the filesystem, partition,
/// write, read back.
const STAGES: [&[&str]; 8] = [
    &["Validating official firmware"],
    &["Extracting firmware"],
    &["Verifying firmware payload"],
    &["Adding root console and NAM support"],
    &["Building root filesystem", "Build complete"],
    &["Preparing partitions"],
    &["Writing rootfs partition"],
    &["Verifying rootfs"],
];

pub fn stage(label: &str) -> Option<usize> {
    STAGES.iter().position(|labels| labels.contains(&label))
}

/// The pinned `device/` assets: bundled resource in a release, repo copy in development.
pub fn device_dir(app: &tauri::AppHandle) -> Option<DeviceDir> {
    let mut candidates: Vec<PathBuf> = vec![];
    if let Ok(res) = app.path().resource_dir() {
        candidates.push(res.join("device"));
    }
    if cfg!(debug_assertions) {
        candidates.push(DeviceDir::repository().0);
    }
    let refs: Vec<&Path> = candidates.iter().map(PathBuf::as_path).collect();
    DeviceDir::locate(&refs)
}

pub fn environment(app: &tauri::AppHandle) -> Environment {
    let release = Release::embedded();
    let dir = device_dir(app);
    let kit_error = match &dir {
        None => Some("The card assets (device/) are missing from this app.".to_string()),
        Some(d) => d
            .verify(&release)
            .err()
            .map(|e| format!("Card assets failed verification: {e}")),
    };
    let default_fw = std::env::var("HOME")
        .ok()
        .map(|h| PathBuf::from(h).join("Downloads/ToneMasterPro_v1_8_58.img"))
        .filter(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned());
    Environment {
        kit_root: dir.map(|d| d.0.to_string_lossy().into_owned()),
        kit_error,
        tools: tmp_sdcard::BUILD_TOOLS
            .iter()
            .map(|t| Tool {
                name: t.to_string(),
                path: tmp_sdcard::util::locate_tool(t)
                    .ok()
                    .map(|p| p.to_string_lossy().into_owned()),
            })
            .collect(),
        default_firmware: default_fw,
        firmware_sha256: release.firmware.sha256,
        platform: std::env::consts::OS.into(),
    }
}

pub fn check_firmware(path: &str) -> Result<Firmware, String> {
    let release = Release::embedded();
    let mut f = std::fs::File::open(path).map_err(|e| format!("{path}: {e}"))?;
    let bytes = f.metadata().map_err(|e| e.to_string())?.len();
    if bytes != release.firmware.bytes {
        return Ok(Firmware {
            path: path.into(),
            bytes,
            sha256: String::new(),
            matches: false,
        });
    }
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let sha256: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(Firmware {
        path: path.into(),
        bytes,
        matches: sha256 == release.firmware.sha256,
        sha256,
    })
}

/// Removable disks; accepted ones that can't hold the card are refused here too.
pub fn list_disks() -> Result<Vec<card::Candidate>, String> {
    if !cfg!(any(target_os = "macos", target_os = "linux")) {
        return Ok(vec![]);
    }
    let release = Release::embedded();
    let mut disks = card::candidates().map_err(|e| e.0)?;
    for d in disks.iter_mut().filter(|d| d.rejected.is_none()) {
        if d.bytes < release.minimum_card_bytes() {
            d.rejected = Some(format!(
                "SD card is too small: {} required",
                tmp_sdcard::util::format_bytes(release.minimum_card_bytes())
            ));
        }
    }
    Ok(disks)
}

/// Overall percent from a helper progress line: rootfs build 0–60 %, then the
/// card-write stages.
pub fn overall(label: &str, done: u64, total: u64) -> f64 {
    let f = if total == 0 {
        1.0
    } else {
        (done as f64 / total as f64).clamp(0.0, 1.0)
    };
    let (start, span) = match label {
        "Preparing partitions" => (60.0, 5.0),
        "Writing rootfs partition" => (65.0, 25.0),
        "Verifying rootfs" => (90.0, 10.0),
        _ => (0.0, 60.0),
    };
    start + span * f
}

/// Turn one helper stdout line into a UI log event; returns an error message if any.
/// `current` tracks the stage the build is in.
fn handle_line(app: &tauri::AppHandle, raw: &str, current: &mut Option<usize>) -> Option<String> {
    let line = raw.trim();
    if line.is_empty() {
        return None;
    }
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        let _ = app.emit(
            "sd://log",
            LogLine {
                line: line.into(),
                percent: None,
                label: None,
                stage: None,
                fraction: None,
            },
        );
        return None;
    };
    if let Some(err) = v.get("error").and_then(Value::as_str) {
        let _ = app.emit(
            "sd://log",
            LogLine {
                line: format!("ERROR: {err}"),
                percent: None,
                label: None,
                stage: None,
                fraction: None,
            },
        );
        return Some(err.to_string());
    }
    if let Some(status) = v.get("status").and_then(Value::as_str) {
        let _ = app.emit(
            "sd://log",
            LogLine {
                line: status.into(),
                percent: None,
                label: None,
                stage: None,
                fraction: None,
            },
        );
        return None;
    }
    let label = v
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let done = v.get("done").and_then(Value::as_u64).unwrap_or(0);
    let total = v.get("total").and_then(Value::as_u64).unwrap_or(0);
    if let Some(s) = stage(&label) {
        *current = Some(s);
    }
    let _ = app.emit(
        "sd://log",
        LogLine {
            line: label.clone(),
            percent: Some(overall(&label, done, total)),
            stage: *current,
            fraction: (total > 0).then(|| done as f64 / total as f64),
            label: Some(label),
        },
    );
    None
}

fn finish(
    app: &tauri::AppHandle,
    code: i32,
    error: Option<String>,
    denied: bool,
    at: Option<usize>,
) {
    let ok = code == 0;
    let outcome = if ok {
        Outcome::Ok
    } else if denied {
        Outcome::Denied
    } else {
        Outcome::Failed
    };
    let message = if ok {
        "Done".to_string()
    } else {
        error.unwrap_or_else(|| {
            if code == 130 {
                "Cancelled".into()
            } else {
                format!("builder exited with code {code}")
            }
        })
    };
    log::info!("card build finished: {outcome:?} code={code} {message}");
    let _ = app.emit(
        "sd://done",
        Finished {
            ok,
            code,
            message,
            outcome,
            stage: if ok || denied { None } else { at.or(Some(0)) },
        },
    );
}

fn helper_base(app: &tauri::AppHandle) -> Result<(PathBuf, DeviceDir), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = device_dir(app).ok_or("card assets (device/) not found")?;
    Ok((exe, dir))
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Write a physical card with administrator rights. The caller already showed the
/// erase confirmation for exactly this identity; the helper refuses if it changed.
pub fn write_card(app: tauri::AppHandle, firmware: String, device: String) -> Result<(), String> {
    log::info!("card build: {device} from {firmware}");
    let target = card::device_info(&device).map_err(|e| e.0)?;
    card::validate_capacity(&target, &Release::embedded()).map_err(|e| e.0)?;
    let (exe, dir) = helper_base(&app)?;
    // Private, randomly named 0700 directory for the log/status files. The
    // elevated command is passed inline (no script file) so nothing in a shared
    // location can be swapped between this point and the administrator prompt.
    let work = tempfile::Builder::new()
        .prefix("tmp-nam-sd-")
        .tempdir()
        .map_err(|e| e.to_string())?
        .keep();
    let log = work.join("build.log");
    let status_file = work.join("status");
    std::fs::write(&log, b"").map_err(|e| e.to_string())?;
    let cmd = [
        sh_quote(&exe.to_string_lossy()),
        "--sdcard-helper write".into(),
        sh_quote(&firmware),
        "--device".into(),
        sh_quote(&target.logical),
        "--yes --json --expect-bytes".into(),
        target.bytes.to_string(),
        "--expect-model".into(),
        sh_quote(&target.model),
        "--device-dir".into(),
        sh_quote(&dir.0.to_string_lossy()),
    ]
    .join(" ");
    let shell = format!(
        "{cmd} > {log} 2>&1; echo $? > {status}",
        log = sh_quote(&log.to_string_lossy()),
        status = sh_quote(&status_file.to_string_lossy()),
    );

    let mut elevate = if cfg!(target_os = "macos") {
        let apple = format!(
            "do shell script \"{}\" with administrator privileges",
            shell.replace('\\', "\\\\").replace('"', "\\\"")
        );
        let mut c = Command::new("osascript");
        c.arg("-e").arg(apple);
        c
    } else {
        let mut c = Command::new("pkexec");
        c.args(["/bin/sh", "-c", &shell]);
        c
    };
    std::thread::spawn(move || {
        let tail_app = app.clone();
        let (tail_log, tail_status) = (log.clone(), status_file.clone());
        let tailer = std::thread::spawn(move || tail(&tail_app, &tail_log, &tail_status));
        let denied = match elevate.output() {
            Ok(o) if !o.status.success() => {
                Some(String::from_utf8_lossy(&o.stderr).trim().to_string())
            }
            Err(e) => Some(e.to_string()),
            _ => None,
        };
        if !status_file.exists() {
            let _ = std::fs::write(&status_file, b"130");
        }
        let (error, at) = tailer.join().unwrap_or((None, None));
        let code: i32 = std::fs::read_to_string(&status_file)
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(1);
        let was_denied = code == 130 && denied.is_some();
        let message = if was_denied {
            Some("Administrator access was not granted; nothing was written".to_string())
        } else {
            error
        };
        finish(&app, code, message, was_denied, at);
        let _ = std::fs::remove_dir_all(&work);
    });
    Ok(())
}

/// Follow the helper log until the status file appears; returns the last error and
/// the stage that was running.
fn tail(app: &tauri::AppHandle, log: &Path, status: &Path) -> (Option<String>, Option<usize>) {
    let mut pos = 0u64;
    let mut carry = String::new();
    let mut error = None;
    let mut current = None;
    loop {
        let done = status.exists();
        if let Ok(mut f) = std::fs::File::open(log) {
            if f.seek(SeekFrom::Start(pos)).is_ok() {
                let mut chunk = String::new();
                if let Ok(n) = f.read_to_string(&mut chunk) {
                    pos += n as u64;
                    carry.push_str(&chunk);
                    while let Some(nl) = carry.find('\n') {
                        let line: String = carry.drain(..=nl).collect();
                        if let Some(e) = handle_line(app, &line, &mut current) {
                            error = Some(e);
                        }
                    }
                }
            }
        }
        if done {
            if let Some(e) = handle_line(app, &carry, &mut current) {
                error = Some(e);
            }
            return (error, current);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overall_progress_is_monotonic_across_stages() {
        let seq = [
            overall("Validating official firmware", 0, 5),
            overall("Building root filesystem", 4, 5),
            overall("Build complete", 5, 5),
            overall("Preparing partitions", 1, 1),
            overall("Writing rootfs partition", 1, 2),
            overall("Verifying rootfs", 2, 2),
        ];
        assert!(seq.windows(2).all(|w| w[0] <= w[1]), "{seq:?}");
        assert_eq!(seq[5], 100.0);
    }

    #[test]
    fn builder_labels_map_to_the_eight_stages() {
        let labels = [
            "Validating official firmware",
            "Extracting firmware",
            "Verifying firmware payload",
            "Adding root console and NAM support",
            "Building root filesystem",
            "Preparing partitions",
            "Writing rootfs partition",
            "Verifying rootfs",
        ];
        for (i, l) in labels.iter().enumerate() {
            assert_eq!(stage(l), Some(i), "{l}");
        }
        assert_eq!(stage("Build complete"), Some(4));
        assert_eq!(stage("Building portable image"), None);
    }

    /// The JSON the frontend reads for `sd://log` and `sd://done` (src/lib/api.ts).
    #[test]
    fn events_match_the_frontend_types() {
        let done = Finished {
            ok: false,
            code: 130,
            message: "m".into(),
            outcome: Outcome::Denied,
            stage: None,
        };
        let v = serde_json::to_value(&done).unwrap();
        assert_eq!(v["outcome"], "denied");
        assert!(v["stage"].is_null());
        let log = LogLine {
            line: "Writing rootfs partition".into(),
            percent: Some(70.0),
            label: Some("Writing rootfs partition".into()),
            stage: Some(6),
            fraction: Some(0.2),
        };
        let v = serde_json::to_value(&log).unwrap();
        assert_eq!(v["stage"], 6);
        assert_eq!(v["fraction"], 0.2);
    }
}
