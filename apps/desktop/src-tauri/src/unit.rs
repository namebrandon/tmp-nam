//! NAM captures on the unit: list, add, remove, per-capture player options.
//!
//! `ConsoleUnit` drives the real card over the USB root console (see `console.rs`)
//! with `unit_helper.py`, pushed to `/tmp` on every connection. `SimUnit` keeps the
//! same contract in memory so the UI runs without hardware (`TMP_NAM_SIM=1`).
//!
//! Write path: the running engine (`tm-stomp-server`) adds and removes captures itself
//! over the unit's HID channel (`hid.rs`), the way Pro Control installs IRs, so the
//! picker updates without an engine restart. An add can carry at most ~65 KB, so it
//! carries a tiny placeholder WAV: the real bytes are pushed over the console first
//! (base64 to `/tmp`), the server registers the name and writes the placeholder, then
//! `install` renames the real file over it (tmp + fsync + rename). The server persists
//! `/data/userIRs.json` about a second later. A2 containers are pushed whole — the
//! player selects the size from `/data/nam/player.json`, keyed by the SHA-256 of the
//! installed bytes.
//!
//! Fallback when the HID channel can't be opened (Pro Control holds it, no hidraw
//! access): register the name in `userIRs.json` BEFORE the file lands (the firmware
//! prunes unregistered files), write the file, then `systemctl restart tm-stomp-server`
//! once so the picker reloads. Never stop/start the server separately:
//! `fmic-platform-ready.target` is `BindsTo=` it and the UI client `Requires=` that
//! target, so a plain stop takes the client down and a later start doesn't bring it
//! back; `restart` propagates. After the restart the registry is re-checked in case the
//! engine rewrote it.
//!
//! A send never fails as a whole: `add` returns an `AddOutcome` saying which files
//! loaded, which the unit dropped, and where it stopped. Whatever an interrupted batch
//! left registered without its real file (no file, or still the placeholder) is removed
//! by `discard_unsent` on the next connection, so the interrupted file is never kept.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::console::{find_port, sh_quote, Console};
use crate::hid::{placeholder_wav, HidSession};

const HELPER: &str = include_str!("unit_helper.py");
const HELPER_PATH: &str = "/tmp/tmpnam_helper.py";
const UPLOAD_PATH: &str = "/tmp/tmpnam_upload.b64";

/// `tm-stomp-server`'s systemd start limit on the card (StartLimitBurst=2,
/// StartLimitIntervalSec=1min), with a margin.
const ENGINE_START_LIMIT: usize = 2;
const ENGINE_START_WINDOW: Duration = Duration::from_secs(63);
/// Longest wait for the engine's own registry write after a start.
const REGISTRY_SETTLE: Duration = Duration::from_secs(8);
/// How long the UI client gets to follow an engine restart before it is started.
const CLIENT_GRACE: Duration = Duration::from_secs(10);

/// Wait between attempts to open the HID channel after one failed.
const HID_RETRY: Duration = Duration::from_secs(60);
/// Longest wait for a restarted engine to answer on HID again.
const HID_REARM: Duration = Duration::from_secs(30);
/// Longest wait for the engine to persist `userIRs.json` after an add or remove
/// (0.5–0.8 s on the unit).
const REGISTRY_PERSIST: Duration = Duration::from_secs(8);
/// Where `register` parks an existing file while the engine writes its placeholder.
const KEEP_PATH: &str = "/data/userIRs/.tmpnam-keep";

/// Engine starts this process made, newest last; shared across reconnects.
static ENGINE_STARTS: Mutex<Vec<Instant>> = Mutex::new(Vec::new());

/// Wait until another engine start fits in the start limit, then record it.
fn pace_engine_start() {
    loop {
        let wait = {
            let mut starts = ENGINE_STARTS.lock().unwrap_or_else(|e| e.into_inner());
            let now = Instant::now();
            starts.retain(|t| now.duration_since(*t) < ENGINE_START_WINDOW);
            if starts.len() < ENGINE_START_LIMIT {
                starts.push(now);
                return;
            }
            ENGINE_START_WINDOW - now.duration_since(starts[0])
        };
        log::info!("pacing engine restart: waiting {wait:?}");
        std::thread::sleep(wait);
    }
}

fn note_engine_start() {
    ENGINE_STARTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(Instant::now());
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ModelMeta {
    pub name: Option<String>,
    pub modeled_by: Option<String>,
    pub gear_make: Option<String>,
    pub gear_model: Option<String>,
    pub gear_type: Option<String>,
    pub tone_type: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Submodel {
    pub architecture: Option<String>,
    pub channels: Option<Value>,
    pub max_value: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ModelInfo {
    pub architecture: Option<String>,
    pub version: Option<String>,
    pub sample_rate: Option<f64>,
    /// First-layer width of a single network (A1): names its size.
    #[serde(default)]
    pub channels: Option<Value>,
    #[serde(default)]
    pub meta: ModelMeta,
    #[serde(default)]
    pub submodels: Vec<Submodel>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct PlayerOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_gain: Option<f64>,
}

/// An omitted patch field keeps the current value; null removes the override.
#[derive(Deserialize, Debug, Default)]
pub struct PlayerOptionsPatch {
    #[serde(default)]
    pub size: OptionChange,
    #[serde(default)]
    pub output_gain: OptionChange,
}

impl PlayerOptionsPatch {
    fn is_empty(&self) -> bool {
        self.size == OptionChange::Keep && self.output_gain == OptionChange::Keep
    }
}

#[derive(Debug, Default, PartialEq)]
pub enum OptionChange {
    #[default]
    Keep,
    Remove,
    Set(f64),
}

impl<'de> Deserialize<'de> for OptionChange {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match Option::<f64>::deserialize(deserializer)? {
            Some(value) => Self::Set(value),
            None => Self::Remove,
        })
    }
}

impl OptionChange {
    fn helper_arg(&self) -> String {
        match self {
            Self::Keep => "=".into(),
            Self::Remove => "-".into(),
            Self::Set(value) => value.to_string(),
        }
    }

    fn apply(&self, value: &mut Option<f64>) {
        match self {
            Self::Keep => {}
            Self::Remove => *value = None,
            Self::Set(next) => *value = Some(*next),
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ModelList<T> {
    pub models: Vec<T>,
    pub settings_error: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UnitModel {
    /// Registry name (`<file minus .wav>`, e.g. `Foo.nam`) — what the IR picker shows.
    pub name: String,
    pub file: String,
    pub bytes: u64,
    pub registered: bool,
    pub present: bool,
    pub sha256: Option<String>,
    pub info: Option<ModelInfo>,
    pub error: Option<String>,
    #[serde(default)]
    pub options: PlayerOptions,
}

#[derive(Serialize, Clone, Debug)]
pub struct UnitInfo {
    pub port: String,
    pub build_id: Option<String>,
    pub dispatch_sha256: Option<String>,
    pub python: Option<String>,
    pub simulated: bool,
    /// Another command holds the connection (a transfer or an engine restart).
    pub busy: bool,
}

pub struct NewModel {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// One step of a send, reported as it happens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SendStep {
    Progress { index: usize, done: u64, total: u64 },
    Sent { index: usize },
    Restarting,
}

pub type Progress<'a> = &'a mut dyn FnMut(SendStep);

/// Why a send stopped before the end.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", content = "message", rename_all = "snake_case")]
pub enum Stop {
    Disconnected,
    DisconnectedDuringRestart,
    Failed(String),
}

/// What a send achieved. `stop` is `None` when every file was sent and the engine
/// restarted; `added` then lists the files the unit loaded.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct AddOutcome {
    pub added: Vec<String>,
    pub failed_after_restart: Vec<String>,
    pub interrupted: Option<String>,
    pub not_sent: Vec<String>,
    pub stop: Option<Stop>,
    /// The batch went through the restart fallback: what was sent loads only after an
    /// engine restart, which then leaves NAM off until the preset is reselected.
    pub needs_restart: bool,
}

impl AddOutcome {
    /// Names that may be registered without a file and need `discard_unsent`.
    pub fn unsent(&self) -> Vec<String> {
        self.interrupted
            .iter()
            .chain(&self.not_sent)
            .cloned()
            .collect()
    }

    fn stopped_at(mut self, names: &[String], index: usize, stop: Stop) -> Self {
        self.interrupted = names.get(index).cloned();
        self.not_sent = names.iter().skip(index + 1).cloned().collect();
        self.stop = Some(stop);
        self
    }

    /// Move the names the unit no longer lists from `added` to `failed_after_restart`.
    fn check_loaded(&mut self, listed: &[UnitModel]) {
        let (ok, dropped) = self.added.drain(..).partition(|n| {
            listed
                .iter()
                .any(|m| &m.name == n && m.present && m.registered)
        });
        self.added = ok;
        self.failed_after_restart = dropped;
    }
}

pub trait Unit: Send {
    fn info(&self) -> UnitInfo;
    fn list(&mut self) -> Result<ModelList<UnitModel>, String>;
    fn add(&mut self, models: Vec<NewModel>, progress: Progress) -> AddOutcome;
    /// Unregister and delete. `Ok(true)` when it took an engine restart.
    fn remove(&mut self, names: &[String]) -> Result<bool, String>;
    /// Register files already on the unit. `Ok(true)` when it took an engine restart.
    fn register(&mut self, names: &[String]) -> Result<bool, String>;
    /// Restart the engine so files sent through the fallback before an interruption
    /// show up.
    fn reload(&mut self) -> Result<(), String>;
    /// Drop what an interrupted batch left registered without its real file (no file,
    /// or the add's placeholder), and the partial upload.
    fn discard_unsent(&mut self, names: &[String]) -> Result<(), String>;
    /// Save options, returning a user-visible warning if invalid settings were recovered.
    fn set_options(
        &mut self,
        sha256: &str,
        opts: &PlayerOptionsPatch,
    ) -> Result<Option<String>, String>;
    /// Cheap liveness check; false means the transport is gone.
    fn alive(&mut self) -> bool;
}

/// Open the real console if a port exists, else the simulator when enabled.
pub fn connect() -> Result<Box<dyn Unit>, String> {
    if std::env::var("TMP_NAM_SIM").is_ok_and(|v| v == "1") {
        return SimUnit::connect().map(|u| Box::new(u) as Box<dyn Unit>);
    }
    let port = find_port().ok_or_else(|| {
        "No unit found. Boot the Tone Master Pro from the NAM SD card and connect USB-C."
            .to_string()
    })?;
    ConsoleUnit::open(&port).map(|u| Box::new(u) as Box<dyn Unit>)
}

/// Normalize a user-facing name into the registry name `<stem>.nam`. ASCII only:
/// the unit's Python runs with an ASCII locale, so a non-ASCII name would reach the
/// registry surrogate-escaped and no longer match its file.
pub fn registry_name(display: &str) -> String {
    let stem = display.trim();
    let stem = stem.strip_suffix(".wav").unwrap_or(stem);
    let stem = stem.strip_suffix(".nam").unwrap_or(stem);
    let cleaned: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() && c != '/' || c == ' ' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let cleaned = cleaned.trim();
    let cleaned = if cleaned.is_empty() {
        "capture"
    } else {
        cleaned
    };
    format!("{cleaned}.nam")
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Validate that bytes look like a NAM model before sending them anywhere.
pub fn validate_nam(bytes: &[u8]) -> Result<ModelInfo, String> {
    let v: Value = serde_json::from_slice(bytes).map_err(|e| format!("not a .nam file: {e}"))?;
    for key in ["architecture", "config", "weights"] {
        if v.get(key).is_none() {
            return Err(format!("not a .nam file: missing \"{key}\""));
        }
    }
    Ok(describe(&v))
}

fn describe(v: &Value) -> ModelInfo {
    let meta = v.get("metadata").cloned().unwrap_or(Value::Null);
    let s = |k: &str| meta.get(k).and_then(|x| x.as_str()).map(str::to_string);
    let mut info = ModelInfo {
        architecture: v
            .get("architecture")
            .and_then(|x| x.as_str())
            .map(str::to_string),
        version: v
            .get("version")
            .and_then(|x| x.as_str())
            .map(str::to_string),
        sample_rate: v.get("sample_rate").and_then(|x| x.as_f64()),
        meta: ModelMeta {
            name: s("name"),
            modeled_by: s("modeled_by"),
            gear_make: s("gear_make"),
            gear_model: s("gear_model"),
            gear_type: s("gear_type"),
            tone_type: s("tone_type"),
        },
        submodels: vec![],
        channels: None,
    };
    if info.architecture.as_deref() != Some("SlimmableContainer") {
        info.channels = v.pointer("/config/layers/0/channels").cloned();
    }
    if info.architecture.as_deref() == Some("SlimmableContainer") {
        if let Some(subs) = v.pointer("/config/submodels").and_then(|x| x.as_array()) {
            for item in subs {
                let child = item.get("model").cloned().unwrap_or(Value::Null);
                info.submodels.push(Submodel {
                    architecture: child
                        .get("architecture")
                        .and_then(|x| x.as_str())
                        .map(str::to_string),
                    channels: child.pointer("/config/layers/0/channels").cloned(),
                    max_value: item
                        .get("max_value")
                        .and_then(|x| x.as_f64())
                        .unwrap_or(1.0),
                });
            }
        }
    }
    info
}

// ---------------------------------------------------------------------------
// Real unit over the USB console
// ---------------------------------------------------------------------------

/// A restart-free operation: `(unit, names, the engine's library)`.
type LiveOp = fn(&mut ConsoleUnit, &[String], &[String]) -> Result<(), String>;

pub struct ConsoleUnit {
    console: Console,
    info: UnitInfo,
    /// The engine's HID channel, opened on first use and kept for the connection: on
    /// macOS a closed exclusive device refuses re-opens for tens of seconds.
    hid: Option<HidSession>,
    /// When opening it last failed; a failed exclusive open re-arms that lockout, so
    /// don't retry before `HID_RETRY`.
    hid_failed: Option<Instant>,
}

fn parse_last_json(out: &str) -> Result<Value, String> {
    let line = out
        .lines()
        .rev()
        .find(|l| l.trim_start().starts_with('{'))
        .ok_or_else(|| format!("unexpected reply from unit: {}", out.trim()))?;
    let v: Value = serde_json::from_str(line).map_err(|e| format!("bad reply: {e}"))?;
    if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
        return Err(err.to_string());
    }
    Ok(v)
}

impl ConsoleUnit {
    pub fn open(port: &str) -> Result<Self, String> {
        let mut console = Console::open(port)?;
        // The helper survives an unplug (it lives in /tmp until the unit reboots):
        // re-sending it costs about a second on every reconnect, so check first.
        let want = sha256_hex(HELPER.as_bytes());
        let have = console
            .run(
                &format!("sha256sum {HELPER_PATH} 2>/dev/null"),
                Duration::from_secs(10),
            )
            .map(|(_, out)| out.contains(&want))
            .unwrap_or(false);
        if !have {
            console.write_text(HELPER_PATH, HELPER)?;
        }
        let mut unit = ConsoleUnit {
            info: UnitInfo {
                port: port.to_string(),
                build_id: None,
                dispatch_sha256: None,
                python: None,
                simulated: false,
                busy: false,
            },
            console,
            hid: None,
            hid_failed: None,
        };
        let v = unit.helper("info", &[], 30)?;
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
        unit.info.build_id = s("build_id");
        unit.info.dispatch_sha256 = s("dispatch_sha256");
        unit.info.python = s("python");
        Ok(unit)
    }

    fn helper(&mut self, cmd: &str, args: &[String], timeout_s: u64) -> Result<Value, String> {
        let mut line = format!("python3 {HELPER_PATH} {cmd}");
        for a in args {
            line.push(' ');
            line.push_str(&sh_quote(a));
        }
        let (_, out) = self.console.run(&line, Duration::from_secs(timeout_s))?;
        parse_last_json(&out)
    }

    /// Restart the audio engine (and, by propagation, the UI client) and wait until
    /// both are active again.
    ///
    /// systemd allows the engine only `ENGINE_START_LIMIT` starts per
    /// `ENGINE_START_WINDOW`; one more and it refuses with `start-limit-hit`, which
    /// also stops the UI client and leaves the unit silent. So restarts are paced,
    /// and a refused one (starts made outside this process, e.g. a previous run)
    /// is retried once the window has passed.
    fn reload_engine(&mut self) -> Result<(), String> {
        let before = self.registry_mtime();
        pace_engine_start();
        if let Err(e) = self
            .console
            .check("systemctl restart tm-stomp-server", Duration::from_secs(90))
        {
            if !self.start_limit_hit() {
                return Err(e);
            }
            log::warn!("engine start limit hit; retrying after the window");
            std::thread::sleep(ENGINE_START_WINDOW);
            note_engine_start();
            self.console.check(
                "systemctl reset-failed tm-stomp-server && systemctl restart tm-stomp-server",
                Duration::from_secs(90),
            )?;
        }
        self.wait_engine_active()?;
        self.wait_registry_rewrite(before);
        self.rearm_hid();
        Ok(())
    }

    /// Re-run the HID handshake after an engine restart. The new engine accepts
    /// reports only once it has opened the gadget, some seconds after systemd calls it
    /// active (writes time out until then), so retry for `HID_REARM`.
    fn rearm_hid(&mut self) {
        let Some(h) = self.hid.as_mut() else {
            return;
        };
        let started = Instant::now();
        let error = loop {
            match h.handshake() {
                Ok(()) => return,
                Err(e) if started.elapsed() > HID_REARM => break e,
                Err(_) => std::thread::sleep(Duration::from_secs(1)),
            }
        };
        self.hid_lost(&error);
    }

    fn registry_mtime(&mut self) -> Option<String> {
        self.console
            .run("stat -c %Y /data/userIRs.json", Duration::from_secs(10))
            .ok()
            .map(|(_, out)| out.trim().to_string())
    }

    /// The engine rewrites `/data/userIRs.json` from what it loaded about a second
    /// after it starts, undoing any registry edit
    /// made in between. Wait for that write (or `REGISTRY_SETTLE`) before the next edit.
    fn wait_registry_rewrite(&mut self, before: Option<String>) {
        let started = Instant::now();
        while started.elapsed() < REGISTRY_SETTLE {
            if before.is_some() && self.registry_mtime() != before {
                std::thread::sleep(Duration::from_secs(1));
                return;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    fn start_limit_hit(&mut self) -> bool {
        self.console
            .run(
                "systemctl show -p Result tm-stomp-server",
                Duration::from_secs(10),
            )
            .is_ok_and(|(_, out)| out.contains("start-limit-hit"))
    }

    /// Wait for the engine and the UI client. The client normally follows the
    /// engine's restart; if a failed start stopped it as a dependency, start it.
    fn wait_engine_active(&mut self) -> Result<(), String> {
        let started = Instant::now();
        let mut kicked_client = false;
        loop {
            // `is-active` exits 0 when *any* unit is active, so read each state.
            let (_, out) = self.console.run(
                "systemctl is-active tm-stomp-server tone-master-stomp-client",
                Duration::from_secs(10),
            )?;
            let states: Vec<&str> = out.split_whitespace().collect();
            if states == ["active", "active"] {
                return Ok(());
            }
            let server_up = states.first() == Some(&"active");
            if server_up && !kicked_client && started.elapsed() > CLIENT_GRACE {
                kicked_client = true;
                self.console.run(
                    "systemctl start tone-master-stomp-client",
                    Duration::from_secs(60),
                )?;
            }
            if started.elapsed() > Duration::from_secs(90) {
                return Err(format!(
                    "audio engine did not come back after restart: {}",
                    states.join(" ")
                ));
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    }

    /// After the files land: sync, restart, re-register in case the engine rewrote
    /// `userIRs.json` while it ran (reloading once more only if that added names), list.
    fn restart_and_list(&mut self, names: &[String]) -> Result<Vec<UnitModel>, String> {
        self.console.check("sync", Duration::from_secs(30))?;
        self.reload_engine()?;
        let added = self.helper("register", names, 60)?;
        if added["added"].as_array().is_some_and(|a| !a.is_empty()) {
            self.console.check("sync", Duration::from_secs(30))?;
            self.reload_engine()?;
        }
        self.list().map(|listed| listed.models)
    }

    /// The engine's library over HID, opening the channel on first use; `None` means
    /// use the restart fallback.
    fn server_library(&mut self) -> Option<Vec<String>> {
        if self.hid.is_none() {
            if self.hid_failed.is_some_and(|t| t.elapsed() < HID_RETRY) {
                return None;
            }
            match HidSession::open() {
                Ok(h) => {
                    log::info!("HID channel open: adds and removes skip the engine restart");
                    self.hid = Some(h);
                }
                Err(e) => {
                    log::warn!("HID channel unavailable, using engine restarts: {e}");
                    self.hid_failed = Some(Instant::now());
                    return None;
                }
            }
        }
        let hid = self.hid.as_mut()?;
        // No answer: the engine may have restarted under the session (outside this
        // app); re-arm once before giving up on HID.
        match hid
            .list()
            .or_else(|_| hid.handshake().and_then(|()| hid.list()))
        {
            Ok(l) => Some(l),
            Err(e) => {
                self.hid_lost(&e);
                None
            }
        }
    }

    fn hid_lost(&mut self, error: &str) {
        log::warn!("HID channel lost: {error}");
        self.hid = None;
        self.hid_failed = Some(Instant::now());
    }

    fn hid(&mut self) -> Result<&mut HidSession, String> {
        self.hid
            .as_mut()
            .ok_or_else(|| "HID channel closed".to_string())
    }

    /// Run `live` with the engine's library. `Ok(true)` when it did the job, `Ok(false)`
    /// when the caller must fall back to an engine restart (no HID channel, or it
    /// failed while the unit stayed connected), `Err` when the unit is gone.
    fn try_live(&mut self, names: &[String], live: LiveOp) -> Result<bool, String> {
        let Some(library) = self.server_library() else {
            return Ok(false);
        };
        match live(self, names, &library) {
            Ok(()) => Ok(true),
            Err(e) if !self.alive() => Err(e),
            Err(e) => {
                self.hid_lost(&e);
                Ok(false)
            }
        }
    }

    /// Wait (on the unit) until the engine's saved `userIRs.json` lists every name in
    /// `present` and none in `absent`.
    fn wait_persisted(&mut self, present: &[String], absent: &[String]) -> Result<(), String> {
        let mut args = vec![REGISTRY_PERSIST.as_secs().to_string()];
        args.extend(present.iter().map(|n| format!("+{n}")));
        args.extend(absent.iter().map(|n| format!("-{n}")));
        self.helper("persisted", &args, REGISTRY_PERSIST.as_secs() + 30)
            .map(|_| ())
    }

    /// Push each file and install it over the console; `before_install` runs between
    /// the two (the restart-free path registers the name there). Stops at the first
    /// failure.
    fn send_files(
        &mut self,
        models: &[NewModel],
        mut out: AddOutcome,
        progress: Progress,
        mut before_install: impl FnMut(&mut Self, &NewModel) -> Result<(), String>,
    ) -> AddOutcome {
        let names: Vec<String> = models.iter().map(|m| m.name.clone()).collect();
        for (i, m) in models.iter().enumerate() {
            let total = m.bytes.len() as u64;
            let sent = self
                .console
                .push_base64(UPLOAD_PATH, &m.bytes, &mut |f| {
                    progress(SendStep::Progress {
                        index: i,
                        done: (f * total as f64) as u64,
                        total,
                    })
                })
                .and_then(|()| before_install(self, m))
                .and_then(|()| {
                    self.helper(
                        "install",
                        &[
                            UPLOAD_PATH.to_string(),
                            m.name.clone(),
                            sha256_hex(&m.bytes),
                        ],
                        120,
                    )
                });
            if let Err(e) = sent {
                let stop = self.stop_reason(e);
                return out.stopped_at(&names, i, stop);
            }
            progress(SendStep::Sent { index: i });
            out.added.push(m.name.clone());
        }
        out
    }

    /// The restart-free send: per file, push the bytes, have the engine add the name
    /// (with a placeholder), then rename the real bytes over the placeholder. A name
    /// the engine already lists is only overwritten (a second add would duplicate it).
    fn add_live(
        &mut self,
        models: &[NewModel],
        mut library: Vec<String>,
        progress: Progress,
    ) -> AddOutcome {
        let mut out = self.send_files(models, AddOutcome::default(), progress, |unit, m| {
            if !library.contains(&m.name) {
                unit.hid()?.add(&m.name, &placeholder_wav())?;
                library.push(m.name.clone());
            }
            Ok(())
        });
        if out.stop.is_none() {
            if let Err(e) = self.wait_persisted(&out.added, &[]) {
                out.stop = Some(self.stop_reason(e));
            }
        }
        out
    }

    /// The fallback send: register every name, write the files, restart the engine.
    fn add_restart(&mut self, models: &[NewModel], progress: Progress) -> AddOutcome {
        let names: Vec<String> = models.iter().map(|m| m.name.clone()).collect();
        let out = AddOutcome {
            needs_restart: true,
            ..AddOutcome::default()
        };
        if let Err(e) = self.helper("register", &names, 60) {
            let stop = self.stop_reason(e);
            return out.stopped_at(&names, 0, stop);
        }
        let mut out = self.send_files(models, out, progress, |_, _| Ok(()));
        if out.stop.is_some() {
            return out;
        }
        progress(SendStep::Restarting);
        match self.restart_and_list(&names) {
            Ok(listed) => out.check_loaded(&listed),
            Err(e) => {
                out.stop = Some(match self.stop_reason(e) {
                    Stop::Disconnected => Stop::DisconnectedDuringRestart,
                    other => other,
                })
            }
        }
        out
    }

    fn remove_live(&mut self, names: &[String], library: &[String]) -> Result<(), String> {
        let loaded: Vec<String> = names
            .iter()
            .filter(|n| library.contains(n))
            .cloned()
            .collect();
        // Later slots shift down after a removal: go from the highest.
        let mut slots: Vec<u32> = library
            .iter()
            .enumerate()
            .filter(|(_, n)| names.contains(n))
            .map(|(i, _)| i as u32 + 1)
            .collect();
        slots.sort_unstable_by(|a, b| b.cmp(a));
        for slot in slots {
            self.hid()?.remove(slot)?;
        }
        // Only names the engine held disappear from its save; the rest (a file without
        // its entry, an entry it never loaded) are the helper's to drop, afterwards.
        self.wait_persisted(&[], &loaded)?;
        self.helper("unregister", names, 60)?;
        Ok(())
    }

    /// Register an existing file: park it, let the engine add the name (writing its
    /// placeholder), then put the file back over the placeholder.
    fn register_live(&mut self, names: &[String], library: &[String]) -> Result<(), String> {
        for n in names.iter().filter(|n| !library.contains(n)) {
            let file = sh_quote(&format!("/data/userIRs/{n}.wav"));
            let keep = sh_quote(KEEP_PATH);
            self.console
                .check(&format!("mv -f {file} {keep}"), Duration::from_secs(10))?;
            let added = self.hid().and_then(|h| h.add(n, &placeholder_wav()));
            self.console.check(
                &format!("mv -f {keep} {file} && sync"),
                Duration::from_secs(30),
            )?;
            added?;
        }
        self.wait_persisted(names, &[])
    }

    fn stop_reason(&mut self, error: String) -> Stop {
        if self.alive() {
            Stop::Failed(error)
        } else {
            Stop::Disconnected
        }
    }
}

impl Unit for ConsoleUnit {
    fn info(&self) -> UnitInfo {
        self.info.clone()
    }

    fn list(&mut self) -> Result<ModelList<UnitModel>, String> {
        let v = self.helper("list", &[], 300)?;
        serde_json::from_value(v).map_err(|e| format!("bad model list: {e}"))
    }

    fn add(&mut self, models: Vec<NewModel>, progress: Progress) -> AddOutcome {
        match self.server_library() {
            Some(library) => self.add_live(&models, library, progress),
            None => self.add_restart(&models, progress),
        }
    }

    fn remove(&mut self, names: &[String]) -> Result<bool, String> {
        if self.try_live(names, Self::remove_live)? {
            return Ok(false);
        }
        self.helper("unregister", names, 60)?;
        self.console.check("sync", Duration::from_secs(30))?;
        self.reload_engine()?;
        Ok(true)
    }

    fn register(&mut self, names: &[String]) -> Result<bool, String> {
        if self.try_live(names, Self::register_live)? {
            return Ok(false);
        }
        self.helper("register", names, 60)?;
        self.console.check("sync", Duration::from_secs(30))?;
        self.reload_engine()?;
        Ok(true)
    }

    fn reload(&mut self) -> Result<(), String> {
        self.reload_engine()
    }

    fn discard_unsent(&mut self, names: &[String]) -> Result<(), String> {
        let placeholder = Some(sha256_hex(&placeholder_wav()));
        let listed = self.list()?;
        let mut orphans = vec![];
        let mut in_picker = false;
        for m in listed.models.iter().filter(|m| names.contains(&m.name)) {
            // A placeholder means the engine added the name but the real bytes never
            // landed: it is in the picker and only the engine (or a restart) drops it.
            let holds_placeholder = m.sha256 == placeholder;
            if !m.present || holds_placeholder {
                orphans.push(m.name.clone());
                in_picker |= holds_placeholder;
            }
        }
        if !orphans.is_empty() && !self.try_live(&orphans, Self::remove_live)? {
            self.helper("unregister", &orphans, 60)?;
            if in_picker {
                self.console.check("sync", Duration::from_secs(30))?;
                self.reload_engine()?;
            }
        }
        self.console.check(
            &format!("rm -f {} && sync", sh_quote(UPLOAD_PATH)),
            Duration::from_secs(30),
        )?;
        Ok(())
    }

    fn set_options(
        &mut self,
        sha256: &str,
        opts: &PlayerOptionsPatch,
    ) -> Result<Option<String>, String> {
        if opts.is_empty() {
            return Ok(None);
        }
        let result = self.helper(
            "opts",
            &[
                sha256.to_string(),
                opts.size.helper_arg(),
                opts.output_gain.helper_arg(),
            ],
            30,
        )?;
        Ok(result
            .get("warning")
            .and_then(Value::as_str)
            .map(str::to_string))
    }

    fn alive(&mut self) -> bool {
        self.console.run("true", Duration::from_secs(3)).is_ok()
    }
}

// ---------------------------------------------------------------------------
// In-memory simulator (TMP_NAM_SIM=1)
// ---------------------------------------------------------------------------

/// Simulator state survives reconnects (a simulated unplug drops the `SimUnit`).
static SIM: Mutex<Option<SimState>> = Mutex::new(None);

struct SimState {
    models: BTreeMap<String, UnitModel>,
    options: BTreeMap<String, PlayerOptions>,
    /// Simulated unplug: the unit is gone until this instant.
    offline_until: Option<Instant>,
}

/// Failure to simulate on the next send (`TMP_NAM_SIM_FAIL`): `disconnect` (mid-file),
/// `restart` (during the engine restart) or `drop` (last file not loaded after it).
fn sim_failure() -> Option<String> {
    std::env::var("TMP_NAM_SIM_FAIL")
        .ok()
        .filter(|v| !v.is_empty())
}

/// Simulate the restart fallback (no HID channel): `TMP_NAM_SIM_RESTART=1`, or the
/// `restart` failure, which only exists there.
fn sim_restart() -> bool {
    std::env::var("TMP_NAM_SIM_RESTART").is_ok_and(|v| v == "1")
        || sim_failure().as_deref() == Some("restart")
}

const SIM_OFFLINE: Duration = Duration::from_secs(6);

pub struct SimUnit;

impl SimUnit {
    pub fn connect() -> Result<Self, String> {
        let mut sim = SIM.lock().map_err(|_| "simulator lock poisoned")?;
        let state = sim.get_or_insert_with(SimState::seeded);
        if state.offline_until.is_some_and(|t| Instant::now() < t) {
            return Err("No unit found (simulated unplug).".into());
        }
        state.offline_until = None;
        Ok(SimUnit)
    }

    fn with<T>(f: impl FnOnce(&mut SimState) -> T) -> T {
        let mut sim = SIM.lock().unwrap_or_else(|e| e.into_inner());
        f(sim.get_or_insert_with(SimState::seeded))
    }

    fn unplug() {
        Self::with(|s| s.offline_until = Some(Instant::now() + SIM_OFFLINE));
    }
}

impl SimState {
    fn seeded() -> Self {
        let mut sim = SimState {
            models: BTreeMap::new(),
            options: BTreeMap::new(),
            offline_until: None,
        };
        let a2 = |name: &str, make: &str, gear: &str| ModelInfo {
            architecture: Some("SlimmableContainer".into()),
            version: Some("0.7.0".into()),
            sample_rate: Some(48000.0),
            meta: ModelMeta {
                name: Some(name.into()),
                gear_make: Some(make.into()),
                gear_model: Some(gear.into()),
                gear_type: Some("amp".into()),
                ..Default::default()
            },
            channels: None,
            submodels: vec![
                Submodel {
                    architecture: Some("WaveNet".into()),
                    channels: Some(3.into()),
                    max_value: 0.5,
                },
                Submodel {
                    architecture: Some("WaveNet".into()),
                    channels: Some(8.into()),
                    max_value: 1.0,
                },
            ],
        };
        let a1 = ModelInfo {
            architecture: Some("WaveNet".into()),
            version: Some("0.5.4".into()),
            sample_rate: Some(48000.0),
            meta: ModelMeta {
                name: Some("Bugera 6262".into()),
                gear_model: Some("6262".into()),
                gear_make: Some("Bugera".into()),
                gear_type: Some("amp".into()),
                ..Default::default()
            },
            submodels: vec![],
            channels: Some(8.into()),
        };
        let flags = std::env::var("TMP_NAM_SIM_FLAGS").is_ok_and(|v| v == "1");
        for (name, info, bytes, registered, present) in [
            (
                "TMP-Badonk-Horizon.nam",
                a2("Badonk Horizon", "Badonk", "Horizon"),
                1_912_340u64,
                true,
                true,
            ),
            (
                "TMP-Peavey-5150-Red-TS9.nam",
                a2("5150 Red + TS9", "Peavey", "5150"),
                2_204_118,
                true,
                true,
            ),
            ("Bugera-6262-feather.nam", a1.clone(), 312_448, true, true),
            ("Lost-Capture.nam", a1.clone(), 0, true, !flags),
            ("Stray-Capture.nam", a1, 298_112, !flags, true),
        ] {
            if !flags && (name == "Lost-Capture.nam" || name == "Stray-Capture.nam") {
                continue;
            }
            sim.models.insert(
                name.into(),
                UnitModel {
                    name: name.into(),
                    file: format!("{name}.wav"),
                    bytes,
                    registered,
                    present,
                    sha256: present.then(|| sha256_hex(name.as_bytes())),
                    info: present.then_some(info),
                    error: None,
                    options: PlayerOptions::default(),
                },
            );
        }
        sim
    }
}

impl Unit for SimUnit {
    fn info(&self) -> UnitInfo {
        UnitInfo {
            port: "simulator".into(),
            build_id: Some("sim-0000".into()),
            dispatch_sha256: Some("efcc3167…".into()),
            python: Some("3.5.6".into()),
            simulated: true,
            busy: false,
        }
    }

    fn list(&mut self) -> Result<ModelList<UnitModel>, String> {
        std::thread::sleep(Duration::from_millis(400));
        Ok(ModelList {
            settings_error: None,
            models: Self::with(|s| {
                s.models
                    .values()
                    .cloned()
                    .map(|mut m| {
                        m.options = m
                            .sha256
                            .as_ref()
                            .and_then(|h| s.options.get(h).cloned())
                            .unwrap_or_default();
                        m
                    })
                    .collect()
            }),
        })
    }

    fn add(&mut self, models: Vec<NewModel>, progress: Progress) -> AddOutcome {
        let fail = sim_failure();
        let names: Vec<String> = models.iter().map(|m| m.name.clone()).collect();
        let mut out = AddOutcome {
            needs_restart: sim_restart(),
            ..AddOutcome::default()
        };
        // Register first, like the real unit: an interrupted batch leaves entries
        // without files until `discard_unsent`.
        Self::with(|s| {
            for n in &names {
                s.models.entry(n.clone()).or_insert_with(|| UnitModel {
                    name: n.clone(),
                    file: format!("{n}.wav"),
                    bytes: 0,
                    registered: true,
                    present: false,
                    sha256: None,
                    info: None,
                    error: None,
                    options: PlayerOptions::default(),
                });
            }
        });
        let cut = models.len().min(2) - 1;
        for (i, m) in models.into_iter().enumerate() {
            let total = m.bytes.len() as u64;
            for step in 1..=10u64 {
                std::thread::sleep(Duration::from_millis(80));
                if fail.as_deref() == Some("disconnect") && i == cut && step == 5 {
                    Self::unplug();
                    return out.stopped_at(&names, i, Stop::Disconnected);
                }
                progress(SendStep::Progress {
                    index: i,
                    done: total * step / 10,
                    total,
                });
            }
            let info = validate_nam(&m.bytes).ok();
            Self::with(|s| {
                s.models.insert(
                    m.name.clone(),
                    UnitModel {
                        file: format!("{}.wav", m.name),
                        bytes: total,
                        registered: true,
                        present: true,
                        sha256: Some(sha256_hex(&m.bytes)),
                        info,
                        error: None,
                        options: PlayerOptions::default(),
                        name: m.name.clone(),
                    },
                )
            });
            progress(SendStep::Sent { index: i });
            out.added.push(m.name);
        }
        if out.needs_restart {
            progress(SendStep::Restarting);
            std::thread::sleep(Duration::from_millis(2500));
        } else {
            std::thread::sleep(Duration::from_millis(600));
        }
        match fail.as_deref() {
            Some("restart") => {
                Self::unplug();
                out.stop = Some(Stop::DisconnectedDuringRestart);
                return out;
            }
            Some("drop") => {
                if let Some(last) = names.last() {
                    Self::with(|s| s.models.remove(last));
                }
            }
            _ => {}
        }
        if let Ok(listed) = self.list() {
            out.check_loaded(&listed.models);
        }
        out
    }

    fn remove(&mut self, names: &[String]) -> Result<bool, String> {
        let restart = sim_restart();
        std::thread::sleep(Duration::from_millis(if restart { 1500 } else { 400 }));
        Self::with(|s| {
            for n in names {
                s.models.remove(n);
            }
        });
        Ok(restart)
    }

    fn register(&mut self, names: &[String]) -> Result<bool, String> {
        let restart = sim_restart();
        std::thread::sleep(Duration::from_millis(if restart { 1500 } else { 400 }));
        Self::with(|s| {
            for n in names {
                if let Some(m) = s.models.get_mut(n) {
                    m.registered = true;
                }
            }
        });
        Ok(restart)
    }

    fn reload(&mut self) -> Result<(), String> {
        std::thread::sleep(Duration::from_millis(1500));
        Ok(())
    }

    fn discard_unsent(&mut self, names: &[String]) -> Result<(), String> {
        Self::with(|s| {
            for n in names {
                if s.models.get(n).is_some_and(|m| !m.present) {
                    s.models.remove(n);
                }
            }
        });
        Ok(())
    }

    fn set_options(
        &mut self,
        sha256: &str,
        opts: &PlayerOptionsPatch,
    ) -> Result<Option<String>, String> {
        Self::with(|s| {
            let current = s.options.entry(sha256.to_string()).or_default();
            opts.size.apply(&mut current.size);
            opts.output_gain.apply(&mut current.output_gain);
            if current.size.is_none() && current.output_gain.is_none() {
                s.options.remove(sha256);
            }
        });
        Ok(None)
    }

    fn alive(&mut self) -> bool {
        Self::with(|s| s.offline_until.is_none_or(|t| Instant::now() >= t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn option_patch_distinguishes_keep_remove_and_set() {
        let patch: PlayerOptionsPatch = serde_json::from_str(r#"{"size":0.0}"#).unwrap();
        assert!(!patch.is_empty());
        assert!(serde_json::from_str::<PlayerOptionsPatch>("{}")
            .unwrap()
            .is_empty());
        assert_eq!(patch.size.helper_arg(), "0");
        assert_eq!(patch.output_gain.helper_arg(), "=");
        let full: PlayerOptionsPatch = serde_json::from_str(r#"{"size":null}"#).unwrap();
        assert_eq!(full.size.helper_arg(), "-");
        assert_eq!(full.output_gain, OptionChange::Keep);
        let gain: PlayerOptionsPatch = serde_json::from_str(r#"{"output_gain":null}"#).unwrap();
        assert_eq!(gain.size, OptionChange::Keep);
        assert_eq!(gain.output_gain.helper_arg(), "-");
        assert!(serde_json::from_str::<PlayerOptionsPatch>(r#"{"size":"0"}"#).is_err());
    }

    #[test]
    fn list_settings_warning_survives_decoding_without_models() {
        let reply =
            parse_last_json(r#"{"models":[],"settings_error":"cannot read settings"}"#).unwrap();
        let listed: ModelList<UnitModel> = serde_json::from_value(reply).unwrap();
        assert!(listed.models.is_empty());
        assert_eq!(
            listed.settings_error.as_deref(),
            Some("cannot read settings")
        );
        let valid: ModelList<UnitModel> = serde_json::from_str(r#"{"models":[]}"#).unwrap();
        assert!(valid.settings_error.is_none());
    }

    #[test]
    fn simulator_preserves_gain_during_size_changes() {
        let mut sim = SimUnit::connect().unwrap();
        let sha = "sim-test-option-patch";
        for (json, size, gain) in [
            (r#"{"output_gain":4.0}"#, None, Some(4.0)),
            (r#"{"size":0.0}"#, Some(0.0), Some(4.0)),
            (r#"{"size":null}"#, None, Some(4.0)),
            (r#"{}"#, None, Some(4.0)),
            (r#"{"output_gain":null}"#, None, None),
        ] {
            let patch = serde_json::from_str(json).unwrap();
            sim.set_options(sha, &patch).unwrap();
            let actual = SimUnit::with(|s| s.options.get(sha).cloned().unwrap_or_default());
            assert_eq!(actual.size, size);
            assert_eq!(actual.output_gain, gain);
        }
    }

    #[test]
    fn registry_name_normalizes() {
        assert_eq!(registry_name("My Amp.nam"), "My Amp.nam");
        assert_eq!(registry_name("My Amp"), "My Amp.nam");
        assert_eq!(registry_name("x.nam.wav"), "x.nam");
        assert_eq!(registry_name("a/b"), "a-b.nam");
        assert_eq!(registry_name("  "), "capture.nam");
        assert_eq!(registry_name("Ampeg B\u{f6}ser.nam"), "Ampeg B-ser.nam");
    }

    #[test]
    fn validate_rejects_non_nam() {
        assert!(validate_nam(b"{}").is_err());
        assert!(validate_nam(b"not json").is_err());
        let ok = br#"{"architecture":"WaveNet","config":{},"weights":[],"version":"0.5.4"}"#;
        assert_eq!(
            validate_nam(ok).unwrap().architecture.as_deref(),
            Some("WaveNet")
        );
    }

    #[test]
    fn describe_reads_container_steps() {
        let v: Value = serde_json::from_str(
            r#"{"architecture":"SlimmableContainer","config":{"submodels":[
              {"max_value":0.3,"model":{"architecture":"WaveNet","config":{"layers":[{"channels":3}]}}},
              {"max_value":1.0,"model":{"architecture":"WaveNet","config":{"layers":[{"channels":8}]}}}
            ]},"weights":[]}"#,
        )
        .unwrap();
        let info = describe(&v);
        assert_eq!(info.submodels.len(), 2);
        assert_eq!(info.submodels[0].max_value, 0.3);
        assert_eq!(info.submodels[1].channels, Some(Value::from(8)));
        assert_eq!(info.channels, None);
        let a1: Value = serde_json::from_str(
            r#"{"architecture":"WaveNet","config":{"layers":[{"channels":8},{"channels":4}]},"weights":[]}"#,
        )
        .unwrap();
        assert_eq!(
            describe(&a1).channels,
            Some(Value::from(8)),
            "A1 width names its size"
        );
    }

    #[test]
    fn parse_reply_surfaces_errors() {
        assert!(parse_last_json("noise\n{\"error\":\"boom\"}\n").is_err());
        let v = parse_last_json("junk\n{\"models\":[]}\n").unwrap();
        assert!(v.get("models").is_some());
    }

    #[test]
    fn sh_quote_escapes_single_quotes() {
        assert_eq!(sh_quote("it's"), "'it'\\''s'");
    }

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn outcome_records_where_a_send_stopped() {
        let mut out = AddOutcome::default();
        out.added.push("a.nam".into());
        let out = out.stopped_at(&names(&["a.nam", "b.nam", "c.nam"]), 1, Stop::Disconnected);
        assert_eq!(out.interrupted.as_deref(), Some("b.nam"));
        assert_eq!(out.not_sent, names(&["c.nam"]));
        assert_eq!(out.unsent(), names(&["b.nam", "c.nam"]));
        let json = serde_json::to_value(&out).unwrap();
        assert_eq!(json["stop"]["kind"], "disconnected");
    }

    #[test]
    fn outcome_splits_dropped_files_after_restart() {
        let model = |name: &str, present: bool| UnitModel {
            name: name.into(),
            file: format!("{name}.wav"),
            bytes: 1,
            registered: true,
            present,
            sha256: None,
            info: None,
            error: None,
            options: PlayerOptions::default(),
        };
        let mut out = AddOutcome {
            added: names(&["a.nam", "b.nam", "c.nam"]),
            ..Default::default()
        };
        out.check_loaded(&[model("a.nam", true), model("b.nam", false)]);
        assert_eq!(out.added, names(&["a.nam"]));
        assert_eq!(out.failed_after_restart, names(&["b.nam", "c.nam"]));
        let failed = serde_json::to_value(Stop::Failed("boom".into())).unwrap();
        assert_eq!(
            failed,
            serde_json::json!({"kind": "failed", "message": "boom"})
        );
    }

    #[test]
    fn simulator_sends_and_discards_unsent_entries() {
        let mut sim = SimUnit::connect().unwrap();
        let bytes = br#"{"architecture":"WaveNet","config":{},"weights":[]}"#.to_vec();
        let mut steps = vec![];
        let out = sim.add(
            vec![NewModel {
                name: "SimTest-Sent.nam".into(),
                bytes,
            }],
            &mut |s| steps.push(s),
        );
        assert_eq!(out.added, names(&["SimTest-Sent.nam"]));
        assert_eq!(steps.last(), Some(&SendStep::Sent { index: 0 }));
        assert!(
            !out.needs_restart,
            "the simulator models the HID path by default"
        );

        // An entry registered without its file (what an unplug leaves behind).
        SimUnit::with(|s| {
            let mut m = s.models["SimTest-Sent.nam"].clone();
            m.name = "SimTest-Orphan.nam".into();
            m.present = false;
            s.models.insert(m.name.clone(), m);
        });
        sim.discard_unsent(&names(&["SimTest-Orphan.nam", "SimTest-Sent.nam"]))
            .unwrap();
        let listed = sim.list().unwrap().models;
        assert!(listed.iter().all(|m| m.name != "SimTest-Orphan.nam"));
        assert!(listed.iter().any(|m| m.name == "SimTest-Sent.nam"));
        assert!(!sim.remove(&names(&["SimTest-Sent.nam"])).unwrap());
    }
}

/// Smoke tests against a real unit (booted card, USB connected):
/// `cargo test -p tmp-nam-companion device_ -- --ignored --nocapture --test-threads=1`
#[cfg(test)]
mod device_tests {
    use super::*;

    fn print_models(models: &[UnitModel]) {
        for m in models {
            println!(
                "  {} registered={} present={} bytes={} arch={:?} subs={} opts={:?} err={:?}",
                m.name,
                m.registered,
                m.present,
                m.bytes,
                m.info.as_ref().and_then(|i| i.architecture.clone()),
                m.info.as_ref().map_or(0, |i| i.submodels.len()),
                m.options,
                m.error
            );
        }
    }

    #[test]
    #[ignore = "needs a unit booted from the NAM card on USB"]
    fn device_read_only() {
        let port = find_port().expect("no TMP console port");
        println!("port: {port}");
        let t0 = std::time::Instant::now();
        let mut unit = ConsoleUnit::open(&port).expect("open console");
        println!("open+helper push: {:?}", t0.elapsed());
        println!("info: {:?}", unit.info());
        let t1 = std::time::Instant::now();
        let models = unit.list().expect("list").models;
        println!("list: {:?} ({} models)", t1.elapsed(), models.len());
        print_models(&models);
        assert!(unit.alive());
    }

    /// Adds, configures and removes a throwaway capture through the HID channel; the
    /// engine never restarts (Pro Control must be closed). With `TMPNAM_PROBE_SELECT=1`
    /// it waits up to 90 s for the capture to be selected on the unit and checks that
    /// the NAM player loaded it.
    #[test]
    #[ignore = "needs a unit booted from the NAM card on USB; writes /data"]
    fn device_write_roundtrip() {
        let name = "TMPNAM-Selftest.nam".to_string();
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../player/tests/fixtures/test_model.nam");
        let bytes = std::fs::read(&fixture).expect("fixture");
        validate_nam(&bytes).expect("fixture is a NAM model");
        let sha = sha256_hex(&bytes);
        let mut unit = ConsoleUnit::open(&find_port().expect("port")).expect("open");
        let engine = engine_identity(&mut unit);

        let t = std::time::Instant::now();
        let out = unit.add(
            vec![NewModel {
                name: name.clone(),
                bytes,
            }],
            &mut |step| println!("  {step:?}"),
        );
        assert_eq!(out.added, vec![name.clone()], "{out:?}");
        assert!(out.stop.is_none(), "{out:?}");
        assert!(!out.needs_restart, "fell back to a restart: {out:?}");
        println!("add: {:?}", t.elapsed());
        let m = unit
            .list()
            .expect("list")
            .models
            .into_iter()
            .find(|m| m.name == name)
            .expect("installed");
        assert!(m.registered && m.present, "{m:?}");
        assert_eq!(
            m.sha256.as_deref(),
            Some(sha.as_str()),
            "installed bytes differ"
        );

        unit.set_options(
            &sha,
            &PlayerOptionsPatch {
                size: OptionChange::Keep,
                output_gain: OptionChange::Set(2.0),
            },
        )
        .expect("opts");
        let m = unit
            .list()
            .expect("list")
            .models
            .into_iter()
            .find(|m| m.name == name)
            .unwrap();
        assert_eq!(m.options.output_gain, Some(2.0));
        unit.set_options(
            &sha,
            &PlayerOptionsPatch {
                size: OptionChange::Remove,
                output_gain: OptionChange::Remove,
            },
        )
        .expect("clear opts");

        if std::env::var_os("TMPNAM_PROBE_SELECT").is_some() {
            println!(">>> select {name} in a user-IR block on the unit now (90 s)");
            let grep = format!(
                "grep -F {} /tmp/nam_dispatch.log | tail -n 3",
                sh_quote(&name)
            );
            let deadline = std::time::Instant::now() + Duration::from_secs(90);
            let mut loaded = String::new();
            while loaded.is_empty() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_secs(2));
                loaded = unit.console.run(&grep, Duration::from_secs(10)).unwrap().1;
            }
            println!("dispatch log:\n{loaded}");
            assert!(
                !loaded.trim().is_empty(),
                "the NAM player never loaded {name}"
            );
        }

        let t = std::time::Instant::now();
        let restarted = unit.remove(std::slice::from_ref(&name)).expect("remove");
        assert!(!restarted, "remove fell back to a restart");
        println!("remove: {:?}", t.elapsed());
        assert_eq!(engine, engine_identity(&mut unit), "the engine restarted");
        assert!(unit
            .list()
            .expect("list")
            .models
            .iter()
            .all(|m| m.name != name));
        let (_, out) = unit
            .console
            .run(
                "systemctl is-active tm-stomp-server tone-master-stomp-client; \
                 grep -c TMPNAM-Selftest /data/userIRs.json /data/nam/player.json; \
                 tail -n 3 /tmp/nam_dispatch.log",
                Duration::from_secs(10),
            )
            .unwrap();
        println!("after:\n{out}");
    }

    /// Engine PID + start time and UI client state: unchanged across the probe means
    /// no restart happened.
    fn engine_identity(unit: &mut ConsoleUnit) -> String {
        unit.console
            .check(
                "systemctl show -p MainPID -p ActiveEnterTimestampMonotonic tm-stomp-server; \
                 systemctl show -p MainPID -p ActiveState tone-master-stomp-client",
                Duration::from_secs(10),
            )
            .expect("systemctl show")
            .trim()
            .to_string()
    }

    fn fixture() -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../player/tests/fixtures/test_model.nam");
        std::fs::read(path).expect("fixture")
    }

    fn find(unit: &mut ConsoleUnit, name: &str) -> Option<UnitModel> {
        unit.list()
            .expect("list")
            .models
            .into_iter()
            .find(|m| m.name == name)
    }

    /// What an unplug mid-send leaves (a registered name without a file, a partial
    /// upload) is removed by `discard_unsent`; a sent file is kept. No restart.
    #[test]
    #[ignore = "needs a unit booted from the NAM card on USB"]
    fn device_discard_unsent() {
        let orphan = "TMPNAM-Selftest-Orphan.nam".to_string();
        let kept = "TMPNAM-Selftest-Kept.nam".to_string();
        let bytes = fixture();
        let mut unit = ConsoleUnit::open(&find_port().expect("port")).expect("open");

        // A batch registered up front; only the first file landed before the "unplug".
        unit.helper("register", &[kept.clone(), orphan.clone()], 60)
            .expect("register");
        unit.console
            .push_base64(UPLOAD_PATH, &bytes, &mut |_| {})
            .expect("push");
        unit.helper(
            "install",
            &[UPLOAD_PATH.to_string(), kept.clone(), sha256_hex(&bytes)],
            120,
        )
        .expect("install");
        unit.console
            .push_base64(UPLOAD_PATH, &bytes[..bytes.len() / 2], &mut |_| {})
            .expect("partial push");
        let o = find(&mut unit, &orphan).expect("orphan listed");
        assert!(o.registered && !o.present, "{o:?}");

        unit.discard_unsent(&[orphan.clone(), kept.clone()])
            .expect("discard");
        assert!(find(&mut unit, &orphan).is_none(), "orphan entry discarded");
        let k = find(&mut unit, &kept).expect("sent file kept");
        assert!(k.registered && k.present, "{k:?}");
        let (_, out) = unit
            .console
            .run(&format!("ls {UPLOAD_PATH} 2>&1"), Duration::from_secs(10))
            .unwrap();
        assert!(
            out.contains("No such file"),
            "partial upload removed: {out}"
        );

        unit.remove(std::slice::from_ref(&kept)).expect("cleanup");
        assert!(find(&mut unit, &kept).is_none());
    }

    /// A file on the card but missing from the registry ("Not registered") is
    /// registered by `register` without an engine restart, keeping its bytes.
    #[test]
    #[ignore = "needs a unit booted from the NAM card on USB"]
    fn device_register_unlisted_file() {
        let name = "TMPNAM-Selftest-Unlisted.nam".to_string();
        let bytes = fixture();
        let mut unit = ConsoleUnit::open(&find_port().expect("port")).expect("open");

        unit.console
            .push_base64(UPLOAD_PATH, &bytes, &mut |_| {})
            .expect("push");
        unit.helper(
            "install",
            &[UPLOAD_PATH.to_string(), name.clone(), sha256_hex(&bytes)],
            120,
        )
        .expect("install without registering");
        let m = find(&mut unit, &name).expect("listed");
        assert!(m.present && !m.registered, "{m:?}");

        let engine = engine_identity(&mut unit);
        let t = std::time::Instant::now();
        let restarted = unit
            .register(std::slice::from_ref(&name))
            .expect("register");
        assert!(!restarted, "register fell back to a restart");
        println!("register: {:?}", t.elapsed());
        let m = find(&mut unit, &name).expect("still listed");
        assert!(m.present && m.registered, "{m:?}");
        assert_eq!(m.sha256.as_deref(), Some(sha256_hex(&bytes).as_str()));
        assert_eq!(engine, engine_identity(&mut unit), "the engine restarted");

        unit.remove(std::slice::from_ref(&name)).expect("cleanup");
        assert!(find(&mut unit, &name).is_none());
    }

    /// After an engine restart (one paced restart), the open HID session is re-armed and
    /// the next add still skips the restart.
    #[test]
    #[ignore = "needs a unit booted from the NAM card on USB, Pro Control closed; restarts the engine once"]
    fn device_hid_survives_engine_restart() {
        let mut unit = ConsoleUnit::open(&find_port().expect("port")).expect("open");
        unit.server_library().expect("HID channel");
        unit.reload().expect("reload");
        let engine = engine_identity(&mut unit);
        let t = std::time::Instant::now();
        assert!(
            unit.server_library().is_some(),
            "HID lost after the restart"
        );
        println!("list after restart: {:?}", t.elapsed());
        assert!(t.elapsed() < Duration::from_secs(4), "list needed a re-arm");
        assert_eq!(engine, engine_identity(&mut unit));
    }

    /// An interruption between the engine's add and the rename leaves the name in the
    /// picker with the placeholder; `discard_unsent` has the engine drop it, no restart.
    #[test]
    #[ignore = "needs a unit booted from the NAM card on USB, Pro Control closed; writes /data"]
    fn device_discard_placeholder() {
        let name = "TMPNAM-Selftest-Placeholder.nam".to_string();
        let mut unit = ConsoleUnit::open(&find_port().expect("port")).expect("open");
        let engine = engine_identity(&mut unit);
        unit.server_library().expect("HID channel");
        unit.hid()
            .unwrap()
            .add(&name, &placeholder_wav())
            .expect("add");
        unit.wait_persisted(std::slice::from_ref(&name), &[])
            .expect("persisted");
        let m = find(&mut unit, &name).expect("listed");
        assert_eq!(m.sha256, Some(sha256_hex(&placeholder_wav())), "{m:?}");

        unit.discard_unsent(std::slice::from_ref(&name))
            .expect("discard");
        assert!(find(&mut unit, &name).is_none(), "placeholder discarded");
        assert!(!unit.server_library().unwrap().contains(&name));
        assert_eq!(engine, engine_identity(&mut unit), "the engine restarted");
    }

    /// Read-only service inspection; set TMPNAM_PROBE to a shell command list.
    #[test]
    #[ignore = "needs a unit booted from the NAM card on USB"]
    fn device_probe() {
        let cmds = std::env::var("TMPNAM_PROBE").expect("TMPNAM_PROBE");
        let mut c = Console::open(&find_port().expect("port")).expect("open");
        let (code, out) = c.run(&cmds, Duration::from_secs(60)).expect("run");
        println!("exit {code}\n{out}");
    }
}
