//! TMP NAM desktop app: Tauri commands over the unit console, Tone3000 and the card builder.
//!
//! Every command that touches the network, the console or a subprocess runs on a
//! blocking worker (`spawn_blocking`) so the webview never stalls. The unit is one
//! shared connection behind a mutex; a failed call drops it and the UI reconnects.

mod console;
mod hid;
mod installs;
mod pending;
mod sdcard;
mod settings;
mod t3k;
mod unit;
mod variants;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, TryLockError};

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, State};

use unit::{
    AddOutcome, ModelInfo, ModelList, NewModel, OptionChange, PlayerOptionsPatch, SendStep, Unit,
    UnitInfo, UnitModel,
};

#[derive(Default, Clone)]
struct AppState {
    unit: Arc<Mutex<Option<Box<dyn Unit>>>>,
    /// Last `UnitInfo`, answered with `busy` while a command holds the unit.
    info: Arc<Mutex<Option<UnitInfo>>>,
    /// Last poll found the unit; logs connect/disconnect once, not every poll.
    was_connected: Arc<AtomicBool>,
}

/// Set by the UI while a send, install or card build runs: closing the window or
/// quitting then asks first (`app://quit-requested`) instead of going ahead.
static QUIT_GUARD: AtomicBool = AtomicBool::new(false);

#[tauri::command]
fn set_quit_guard(on: bool) {
    QUIT_GUARD.store(on, Ordering::SeqCst);
}

/// "Quit Anyway".
#[tauri::command]
fn quit_now(app: tauri::AppHandle) {
    QUIT_GUARD.store(false, Ordering::SeqCst);
    log::warn!("quit while an operation was running");
    app.exit(0);
}

/// Quit from the app menu or ⌘Q: ask first while the guard is on. The stock Quit
/// item ends the app without an exit event the guard could catch (macOS), so the
/// menu's Quit is this item instead.
fn request_quit(app: &tauri::AppHandle) {
    if QUIT_GUARD.load(Ordering::SeqCst) {
        let _ = app.emit("app://quit-requested", ());
    } else {
        app.exit(0);
    }
}

/// The macOS menu bar: the standard items, with Quit routed through `request_quit`.
#[cfg(target_os = "macos")]
fn app_menu(app: &tauri::AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem as P, Submenu};
    let quit = MenuItem::with_id(app, "quit", "Quit TMP NAM", true, Some("CmdOrCtrl+Q"))?;
    let main = Submenu::with_items(
        app,
        "TMP NAM",
        true,
        &[
            &P::about(app, Some("About TMP NAM"), None)?,
            &P::separator(app)?,
            &P::services(app, None)?,
            &P::separator(app)?,
            &P::hide(app, None)?,
            &P::hide_others(app, None)?,
            &P::show_all(app, None)?,
            &P::separator(app)?,
            &quit,
        ],
    )?;
    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &P::undo(app, None)?,
            &P::redo(app, None)?,
            &P::separator(app)?,
            &P::cut(app, None)?,
            &P::copy(app, None)?,
            &P::paste(app, None)?,
            &P::select_all(app, None)?,
        ],
    )?;
    let window = Submenu::with_items(
        app,
        "Window",
        true,
        &[
            &P::minimize(app, None)?,
            &P::maximize(app, None)?,
            &P::separator(app)?,
            &P::close_window(app, None)?,
        ],
    )?;
    Menu::with_items(app, &[&main, &edit, &window])
}

/// Log file name (without `.log`) in the platform log directory.
const LOG_FILE: &str = "tmp-nam";

/// `op://event`: one step of the running send or install; the UI folds them into
/// per-item rows.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "phase", rename_all = "snake_case")]
enum OpEvent {
    Download { index: usize, done: u64, total: u64 },
    Send { index: usize, done: u64, total: u64 },
    Sent { index: usize },
    Restart,
}

impl From<SendStep> for OpEvent {
    fn from(s: SendStep) -> Self {
        match s {
            SendStep::Progress { index, done, total } => OpEvent::Send { index, done, total },
            SendStep::Sent { index } => OpEvent::Sent { index },
            SendStep::Restarting => OpEvent::Restart,
        }
    }
}

/// A command error the UI can branch on (`code`) as well as show (`message`).
#[derive(Serialize, Debug, PartialEq)]
struct ApiError {
    code: &'static str,
    message: String,
}

/// Classify a Tone3000 error string for the sign-in and list states. Messages never
/// carry the token (see `t3k::http_err`), so they are safe to log.
fn t3k_error(message: String) -> ApiError {
    log::warn!("tone3000: {message}");
    let m = message.to_ascii_lowercase();
    let code = if m.contains("invalid_client")
        || m.contains("unauthorized_client")
        || m.contains("token exchange failed")
        || m.contains("http 400")
        || m.contains("http 401")
        || m.contains("t3k_pub_")
    {
        "key_rejected"
    } else if m.contains("refused the sign-in") {
        "declined"
    } else if m.contains("cancelled") {
        "cancelled"
    } else if m.contains("timed out") || m.contains("timeout") {
        "timeout"
    } else if m.contains("network error") {
        "network"
    } else if m.contains("not signed in") || m.contains("session expired") {
        "signed_out"
    } else {
        "other"
    };
    ApiError { code, message }
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
}

/// Run `f` against the connected unit, dropping the connection if it fails so the
/// next poll reconnects cleanly.
fn with_unit<T>(
    state: &AppState,
    f: impl FnOnce(&mut dyn Unit) -> Result<T, String>,
) -> Result<T, String> {
    let mut guard = state.unit.lock().map_err(|_| "unit lock poisoned")?;
    let unit = guard.as_mut().ok_or("No unit connected")?;
    let result = f(unit.as_mut());
    if result.is_err() && !unit.alive() {
        *guard = None;
    }
    result
}

// ── Unit ────────────────────────────────────────────────────────────────────

#[tauri::command]
async fn unit_connect(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<UnitInfo, String> {
    let state = state.inner().clone();
    blocking(move || {
        let mut guard = match state.unit.try_lock() {
            Ok(g) => g,
            // A transfer or restart holds the unit: answer at once instead of
            // queueing the 3 s poll behind it.
            Err(TryLockError::WouldBlock) => {
                let cached = state.info.lock().map_err(|_| "info lock poisoned")?.clone();
                return cached
                    .map(|i| UnitInfo { busy: true, ..i })
                    .ok_or_else(|| "No unit connected".to_string());
            }
            Err(TryLockError::Poisoned(_)) => return Err("unit lock poisoned".into()),
        };
        if let Some(u) = guard.as_mut() {
            if u.alive() {
                return Ok(u.info());
            }
        }
        *guard = None;
        if let Ok(mut i) = state.info.lock() {
            *i = None;
        }
        let mut u = match unit::connect() {
            Ok(u) => u,
            Err(e) => {
                if state.was_connected.swap(false, Ordering::SeqCst) {
                    log::info!("unit disconnected: {e}");
                }
                return Err(e);
            }
        };
        let unsent = pending::load(&app);
        if !unsent.is_empty() {
            match u.discard_unsent(&unsent) {
                Ok(()) => {
                    log::info!("discarded an interrupted send: {unsent:?}");
                    pending::save(&app, &[]);
                }
                Err(e) => log::warn!("discarding an interrupted send failed: {e}"),
            }
        }
        let info = u.info();
        if !state.was_connected.swap(true, Ordering::SeqCst) {
            log::info!(
                "unit connected: {} build={} player={}{}",
                info.port,
                info.build_id.as_deref().unwrap_or("?"),
                info.dispatch_sha256.as_deref().unwrap_or("?"),
                if info.simulated { " (simulator)" } else { "" }
            );
        }
        *guard = Some(u);
        if let Ok(mut i) = state.info.lock() {
            *i = Some(info.clone());
        }
        Ok(info)
    })
    .await
}

/// A capture on the unit, with where it came from when Tone3000 installed it.
#[derive(Serialize)]
struct Capture {
    #[serde(flatten)]
    model: UnitModel,
    source: Option<installs::Install>,
}

#[tauri::command]
async fn unit_list(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<ModelList<Capture>, String> {
    let state = state.inner().clone();
    blocking(move || {
        let models = with_unit(&state, |u| u.list())?;
        let record = installs::load(&app);
        Ok(ModelList {
            settings_error: models.settings_error,
            models: models
                .models
                .into_iter()
                .map(|model| Capture {
                    source: model.sha256.as_ref().and_then(|h| record.get(h).cloned()),
                    model,
                })
                .collect(),
        })
    })
    .await
}

/// A `.nam` file checked before sending, for the "Add captures" sheet.
#[derive(Serialize)]
struct Inspected {
    path: String,
    /// Registry name it will get on the unit.
    name: String,
    bytes: u64,
    info: Option<ModelInfo>,
    error: Option<String>,
}

#[tauri::command]
async fn nam_inspect(paths: Vec<String>) -> Result<Vec<Inspected>, String> {
    blocking(move || {
        Ok(paths
            .into_iter()
            .map(|path| {
                let stem = std::path::Path::new(&path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let (bytes, checked) = match std::fs::read(&path) {
                    Ok(b) => (b.len() as u64, unit::validate_nam(&b)),
                    Err(e) => (0, Err(e.to_string())),
                };
                Inspected {
                    name: unit::registry_name(&stem),
                    bytes,
                    error: checked.as_ref().err().cloned(),
                    info: checked.ok(),
                    path,
                }
            })
            .collect())
    })
    .await
}

#[derive(Deserialize)]
struct AddFile {
    path: String,
    name: Option<String>,
}

/// Send models to the unit, streaming `op://event`. The batch is recorded as
/// pending first (`pending.rs`) so whatever an unplug, quit or crash leaves
/// registered without a file is discarded on the next connection.
fn send(
    app: &tauri::AppHandle,
    state: &AppState,
    models: Vec<NewModel>,
) -> Result<AddOutcome, String> {
    let mut guard = state.unit.lock().map_err(|_| "unit lock poisoned")?;
    let unit = guard.as_mut().ok_or("No unit connected")?;
    let before = pending::load(app);
    let names: Vec<String> = models.iter().map(|m| m.name.clone()).collect();
    pending::save(app, &pending::union(&before, &names));
    let out = unit.add(models, &mut |step| {
        let _ = app.emit("op://event", OpEvent::from(step));
    });
    log::info!(
        "send: added={:?} failed_after_restart={:?} interrupted={:?} not_sent={:?} stop={:?}",
        out.added,
        out.failed_after_restart,
        out.interrupted,
        out.not_sent,
        out.stop
    );
    let unsent = out.unsent();
    let alive = unit.alive();
    let discarded = unsent.is_empty()
        || (alive
            && unit
                .discard_unsent(&unsent)
                .inspect_err(|e| log::warn!("discarding unsent entries failed: {e}"))
                .is_ok());
    pending::save(
        app,
        &if discarded {
            before
        } else {
            pending::union(&before, &unsent)
        },
    );
    if !alive {
        *guard = None;
    }
    Ok(out)
}

#[tauri::command]
async fn unit_add_files(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    files: Vec<AddFile>,
) -> Result<AddOutcome, String> {
    let state = state.inner().clone();
    blocking(move || {
        let mut models = vec![];
        for f in files {
            let bytes = std::fs::read(&f.path).map_err(|e| format!("{}: {e}", f.path))?;
            unit::validate_nam(&bytes).map_err(|e| format!("{}: {e}", f.path))?;
            let stem = f.name.unwrap_or_else(|| {
                std::path::Path::new(&f.path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            });
            models.push(NewModel {
                name: unit::registry_name(&stem),
                bytes,
            });
        }
        send(&app, &state, models)
    })
    .await
}

/// `true` when the command took an engine restart (no HID channel): NAM then stays off
/// until the preset is reselected.
#[tauri::command]
async fn unit_remove(state: State<'_, AppState>, names: Vec<String>) -> Result<bool, String> {
    let state = state.inner().clone();
    log::info!("remove {names:?}");
    blocking(move || with_unit(&state, |u| u.remove(&names)))
        .await
        .inspect_err(|e| log::warn!("remove failed: {e}"))
}

#[tauri::command]
async fn unit_register(state: State<'_, AppState>, names: Vec<String>) -> Result<bool, String> {
    let state = state.inner().clone();
    log::info!("register {names:?}");
    blocking(move || with_unit(&state, |u| u.register(&names)))
        .await
        .inspect_err(|e| log::warn!("register failed: {e}"))
}

#[tauri::command]
async fn unit_reload(state: State<'_, AppState>) -> Result<(), String> {
    let state = state.inner().clone();
    blocking(move || with_unit(&state, |u| u.reload())).await
}

#[tauri::command]
async fn unit_set_options(
    state: State<'_, AppState>,
    sha256: String,
    options: PlayerOptionsPatch,
) -> Result<Option<String>, String> {
    if let OptionChange::Set(s) = options.size {
        if !(0.0..=1.0).contains(&s) {
            return Err("size must be between 0 and 1".into());
        }
    }
    if let OptionChange::Set(g) = options.output_gain {
        if !(0.0..=8.0).contains(&g) {
            return Err("output gain must be between 0 and 8".into());
        }
    }
    let state = state.inner().clone();
    blocking(move || with_unit(&state, |u| u.set_options(&sha256, &options))).await
}

// ── Settings ────────────────────────────────────────────────────────────────

#[tauri::command]
fn settings_get(app: tauri::AppHandle) -> settings::Settings {
    settings::load(&app)
}

#[tauri::command]
fn settings_set(app: tauri::AppHandle, settings: settings::Settings) -> Result<(), String> {
    settings::save(&app, &settings)
}

#[tauri::command]
fn variants_list() -> &'static [variants::Variant] {
    variants::VARIANTS
}

// ── Tone3000 ────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct T3kStatus {
    has_key: bool,
    linked: bool,
    username: Option<String>,
}

#[tauri::command]
async fn t3k_status(app: tauri::AppHandle) -> Result<T3kStatus, String> {
    blocking(move || {
        let key = settings::load(&app).t3k_key;
        if key.is_empty() {
            return Ok(T3kStatus {
                has_key: false,
                linked: false,
                username: None,
            });
        }
        if t3k::load_tokens(&app).is_none() {
            return Ok(T3kStatus {
                has_key: true,
                linked: false,
                username: None,
            });
        }
        // Tokens on disk mean signed in, even offline: the tone list then fails
        // into the network error rather than asking to sign in again.
        let username = t3k::Session::new(&app, &key)
            .ok()
            .and_then(|mut s| s.username());
        Ok(T3kStatus {
            has_key: true,
            linked: true,
            username,
        })
    })
    .await
}

#[tauri::command]
async fn t3k_link(app: tauri::AppHandle) -> Result<Option<String>, ApiError> {
    blocking(move || {
        let key = settings::load(&app).t3k_key;
        if key.is_empty() {
            return Err("Add your Tone3000 key in Settings first".into());
        }
        t3k::link(&app, &key)?;
        Ok(t3k::Session::new(&app, &key)?.username())
    })
    .await
    .map_err(t3k_error)
}

#[tauri::command]
fn t3k_open_link_again() -> Result<(), String> {
    t3k::open_link_again()
}

#[tauri::command]
fn t3k_open_site() -> Result<(), String> {
    t3k::open_browser("https://www.tone3000.com")
}

#[tauri::command]
fn t3k_cancel_link() {
    t3k::CANCEL_LINK.store(true, std::sync::atomic::Ordering::SeqCst);
}

#[tauri::command]
fn t3k_unlink(app: tauri::AppHandle) -> Result<(), String> {
    t3k::forget(&app)
}

#[tauri::command]
async fn t3k_tones(app: tauri::AppHandle) -> Result<Vec<t3k::T3kTone>, ApiError> {
    blocking(move || {
        let s = settings::load(&app);
        let mut session = t3k::Session::new(&app, &s.t3k_key)?;
        t3k::collect(&mut session)
    })
    .await
    .map_err(t3k_error)
}

#[derive(Deserialize)]
struct T3kPick {
    model_url: String,
    ir_name: String,
    tone_id: serde_json::Value,
    model_id: serde_json::Value,
    variant: Option<String>,
}

/// Download each pick, then send them all with one engine restart. A download that
/// fails stops before anything reaches the unit.
#[tauri::command]
async fn t3k_install(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    picks: Vec<T3kPick>,
) -> Result<AddOutcome, String> {
    let state = state.inner().clone();
    blocking(move || {
        let s = settings::load(&app);
        let mut session = t3k::Session::new(&app, &s.t3k_key)?;
        let names: Vec<String> = picks
            .iter()
            .map(|p| unit::registry_name(&p.ir_name))
            .collect();
        let mut models = vec![];
        let mut record = vec![];
        for (index, p) in picks.iter().enumerate() {
            let _ = app.emit(
                "op://event",
                OpEvent::Download {
                    index,
                    done: 0,
                    total: 1,
                },
            );
            let fetched = session.download(&p.model_url).and_then(|bytes| {
                unit::validate_nam(&bytes).map_err(|e| format!("{}: {e}", p.ir_name))?;
                Ok(bytes)
            });
            let bytes = match fetched {
                Ok(b) => b,
                Err(e) => {
                    return Ok(AddOutcome {
                        interrupted: names.get(index).cloned(),
                        not_sent: names.iter().skip(index + 1).cloned().collect(),
                        stop: Some(unit::Stop::Failed(e)),
                        ..Default::default()
                    })
                }
            };
            let _ = app.emit(
                "op://event",
                OpEvent::Download {
                    index,
                    done: 1,
                    total: 1,
                },
            );
            record.push((
                names[index].clone(),
                unit::sha256_hex(&bytes),
                installs::Install {
                    tone_id: p.tone_id.clone(),
                    model_id: p.model_id.clone(),
                    variant: p.variant.clone(),
                },
            ));
            models.push(NewModel {
                name: names[index].clone(),
                bytes,
            });
        }
        let out = send(&app, &state, models)?;
        let unsent = out.unsent();
        let landed: Vec<(String, installs::Install)> = record
            .into_iter()
            .filter(|(name, _, _)| !unsent.contains(name))
            .map(|(_, sha, i)| (sha, i))
            .collect();
        installs::record(&app, landed)?;
        Ok(out)
    })
    .await
}

// ── SD card ─────────────────────────────────────────────────────────────────

#[tauri::command]
async fn sd_environment(app: tauri::AppHandle) -> Result<sdcard::Environment, String> {
    blocking(move || Ok(sdcard::environment(&app))).await
}

#[tauri::command]
async fn sd_check_firmware(path: String) -> Result<sdcard::Firmware, String> {
    blocking(move || sdcard::check_firmware(&path)).await
}

#[tauri::command]
async fn sd_list_disks() -> Result<Vec<tmp_sdcard::card::Candidate>, String> {
    blocking(sdcard::list_disks).await
}

#[tauri::command]
async fn sd_write_card(
    app: tauri::AppHandle,
    firmware: String,
    device: String,
) -> Result<(), String> {
    blocking(move || sdcard::write_card(app, firmware, device)).await
}

#[tauri::command]
fn sd_open_privacy_settings() -> Result<(), String> {
    sdcard::open_privacy_settings()
}

/// Text for "Copy Diagnostics": versions, platform, the unit, settings without the
/// key, and the end of the log.
#[tauri::command]
async fn diagnostics(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<String, String> {
    let info = state.info.lock().map_err(|_| "info lock poisoned")?.clone();
    blocking(move || {
        let s = settings::load(&app);
        let mut out = vec![
            format!("TMP NAM {}", app.package_info().version),
            format!(
                "Platform: {} {}",
                std::env::consts::OS,
                std::env::consts::ARCH
            ),
            match info {
                Some(i) => format!(
                    "Unit: {}{} · card build {} · NAM player {} · Python {}",
                    i.port,
                    if i.simulated { " (simulator)" } else { "" },
                    i.build_id.as_deref().unwrap_or("unknown"),
                    i.dispatch_sha256.as_deref().unwrap_or("unknown"),
                    i.python.as_deref().unwrap_or("unknown"),
                ),
                None => "Unit: not connected".into(),
            },
            format!(
                "Tone3000: key {} · signed in {} · allowed {}",
                if s.t3k_key.is_empty() {
                    "not set"
                } else {
                    "set"
                },
                if t3k::load_tokens(&app).is_some() {
                    "yes"
                } else {
                    "no"
                },
                s.variants.join(", ")
            ),
        ];
        let log = app
            .path()
            .app_log_dir()
            .map(|d| d.join(format!("{LOG_FILE}.log")))
            .map_err(|e| e.to_string())?;
        out.push(format!("Log: {}", log.display()));
        out.push(String::new());
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        out.extend(
            lines[lines.len().saturating_sub(80)..]
                .iter()
                .map(|l| l.to_string()),
        );
        Ok(out.join("\n"))
    })
    .await
}

fn logger() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    use tauri_plugin_log::{RotationStrategy, Target, TargetKind};
    tauri_plugin_log::Builder::new()
        .targets([
            Target::new(TargetKind::Stdout),
            Target::new(TargetKind::LogDir {
                file_name: Some(LOG_FILE.into()),
            }),
        ])
        .level(log::LevelFilter::Info)
        // HTTP internals stay out of the log (no request headers, no tokens).
        .level_for("ureq", log::LevelFilter::Warn)
        .level_for("rustls", log::LevelFilter::Warn)
        .max_file_size(2 * 1024 * 1024)
        .rotation_strategy(RotationStrategy::KeepSome(3))
        .build()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    #[cfg(target_os = "macos")]
    let builder = builder.menu(app_menu);
    builder
        .plugin(logger())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            unit_connect,
            unit_list,
            nam_inspect,
            unit_add_files,
            unit_remove,
            unit_register,
            unit_reload,
            unit_set_options,
            settings_get,
            settings_set,
            variants_list,
            t3k_status,
            t3k_link,
            t3k_open_link_again,
            t3k_open_site,
            t3k_cancel_link,
            t3k_unlink,
            t3k_tones,
            t3k_install,
            sd_environment,
            sd_check_firmware,
            sd_list_disks,
            sd_write_card,
            sd_open_privacy_settings,
            diagnostics,
            set_quit_guard,
            quit_now,
        ])
        .on_menu_event(|app, event| {
            if event.id() == "quit" {
                request_quit(app);
            }
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if QUIT_GUARD.load(Ordering::SeqCst) {
                    api.prevent_close();
                    let _ = window.emit("app://quit-requested", ());
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building TMP NAM")
        .run(|app, event| {
            // ⌘Q / the app menu's Quit: no window close event, so guard here too.
            if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
                if code.is_none() && QUIT_GUARD.load(Ordering::SeqCst) {
                    api.prevent_exit();
                    let _ = app.emit("app://quit-requested", ());
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn op_events_serialize_with_their_phase() {
        let e = OpEvent::from(SendStep::Progress {
            index: 1,
            done: 10,
            total: 20,
        });
        assert_eq!(
            serde_json::to_value(e).unwrap(),
            serde_json::json!({"phase": "send", "index": 1, "done": 10, "total": 20})
        );
        assert_eq!(
            serde_json::to_value(OpEvent::from(SendStep::Restarting)).unwrap(),
            serde_json::json!({"phase": "restart"})
        );
    }

    /// The JSON the frontend reads (apps/desktop/src/lib/api.ts): field names and tags.
    #[test]
    fn payloads_match_the_frontend_types() {
        let out = AddOutcome {
            added: vec!["a.nam".into()],
            failed_after_restart: vec!["b.nam".into()],
            interrupted: Some("c.nam".into()),
            not_sent: vec!["d.nam".into()],
            stop: Some(unit::Stop::Failed("boom".into())),
            needs_restart: true,
        };
        assert_eq!(
            serde_json::to_value(&out).unwrap(),
            serde_json::json!({
                "added": ["a.nam"],
                "failed_after_restart": ["b.nam"],
                "interrupted": "c.nam",
                "not_sent": ["d.nam"],
                "stop": {"kind": "failed", "message": "boom"},
                "needs_restart": true,
            })
        );
        assert_eq!(
            serde_json::to_value(unit::Stop::DisconnectedDuringRestart).unwrap(),
            serde_json::json!({"kind": "disconnected_during_restart"})
        );

        let capture = Capture {
            model: UnitModel {
                name: "x.nam".into(),
                file: "x.nam.wav".into(),
                bytes: 1,
                registered: true,
                present: true,
                sha256: Some("ab".into()),
                info: None,
                error: None,
                options: unit::PlayerOptions::default(),
            },
            source: Some(installs::Install {
                tone_id: 7.into(),
                model_id: 9.into(),
                variant: Some("a1-feather".into()),
            }),
        };
        let v = serde_json::to_value(&capture).unwrap();
        assert_eq!(v["name"], "x.nam", "UnitModel fields are flattened");
        assert_eq!(
            v["source"],
            serde_json::json!({"tone_id": 7, "model_id": 9, "variant": "a1-feather"})
        );

        let inspected = Inspected {
            path: "/x.nam".into(),
            name: "x.nam".into(),
            bytes: 0,
            info: None,
            error: Some("bad".into()),
        };
        let v = serde_json::to_value(&inspected).unwrap();
        for k in ["path", "name", "bytes", "info", "error"] {
            assert!(v.get(k).is_some(), "Inspected.{k}");
        }

        let info = UnitInfo {
            port: "p".into(),
            build_id: None,
            dispatch_sha256: None,
            python: None,
            simulated: false,
            busy: true,
        };
        assert_eq!(serde_json::to_value(&info).unwrap()["busy"], true);

        let pick: T3kPick = serde_json::from_value(serde_json::json!({
            "model_url": "https://www.tone3000.com/m",
            "ir_name": "x.nam",
            "tone_id": 1,
            "model_id": "m1",
            "variant": null,
        }))
        .unwrap();
        assert!(pick.variant.is_none());
    }

    #[test]
    fn t3k_errors_are_classified() {
        let code = |m: &str| t3k_error(m.into()).code;
        assert_eq!(
            code("Tone3000 refused the sign-in: invalid_client"),
            "key_rejected"
        );
        assert_eq!(code("Tone3000 returned HTTP 401: nope"), "key_rejected");
        assert_eq!(
            code("The Tone3000 key should start with t3k_pub_"),
            "key_rejected"
        );
        assert_eq!(
            code("Tone3000 refused the sign-in: access_denied"),
            "declined"
        );
        assert_eq!(code("Sign-in cancelled"), "cancelled");
        assert_eq!(
            code("Timed out waiting for the Tone3000 sign-in"),
            "timeout"
        );
        assert_eq!(code("network error: dns"), "network");
        assert_eq!(code("Not signed in to Tone3000"), "signed_out");
        assert_eq!(code("something else"), "other");
    }
}
