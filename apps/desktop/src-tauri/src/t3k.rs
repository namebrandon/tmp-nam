//! Tone3000 sync, run on the desktop instead of on the pedal.
//!
//! Same API and selection rules as `device/helpers/t3k_sync.py`, with two
//! changes: OAuth uses a loopback redirect (`http://127.0.0.1:8766/cb`) and the system
//! browser instead of the LAN relay, and models download here and reach the unit over
//! the USB console — the pedal never needs Wi-Fi. The user's publishable key is the
//! OAuth client id; tokens live in `<app data dir>/t3k_tokens.json` (mode 0600).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::Manager;

use crate::variants;

const API: &str = "https://www.tone3000.com/api/v1";
const LISTEN_PORT: u16 = 8766;
const MAX_MODEL_BYTES: u64 = 64 * 1024 * 1024;

pub static CANCEL_LINK: AtomicBool = AtomicBool::new(false);
/// The authorize URL of the sign-in in progress, for "Open Page Again".
static LINK_URL: Mutex<Option<String>> = Mutex::new(None);

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Tokens {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

fn tokens_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("t3k_tokens.json"))
}

pub fn load_tokens(app: &tauri::AppHandle) -> Option<Tokens> {
    let p = tokens_path(app).ok()?;
    serde_json::from_slice(&std::fs::read(p).ok()?).ok()
}

fn save_tokens(app: &tauri::AppHandle, tok: &Tokens) -> Result<(), String> {
    let p = tokens_path(app)?;
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(tok).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
}

pub fn forget(app: &tauri::AppHandle) -> Result<(), String> {
    let p = tokens_path(app)?;
    if p.exists() {
        std::fs::remove_file(p).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn random(n: usize) -> Vec<u8> {
    let mut v = vec![0u8; n];
    getrandom::getrandom(&mut v).expect("OS randomness");
    v
}

fn http_err(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, resp) => {
            let body = resp.into_string().unwrap_or_default();
            format!(
                "Tone3000 returned HTTP {code}: {}",
                body.chars().take(300).collect::<String>()
            )
        }
        other => format!("network error: {other}"),
    }
}

fn token_request(form: &[(&str, &str)]) -> Result<Tokens, String> {
    let v: Value = ureq::post(&format!("{API}/oauth/token"))
        .timeout(Duration::from_secs(60))
        .send_form(form)
        .map_err(http_err)?
        .into_json()
        .map_err(|e| e.to_string())?;
    serde_json::from_value(v.clone()).map_err(|_| format!("token exchange failed: {v}"))
}

pub fn open_browser(url: &str) -> Result<(), String> {
    let mut cmd = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else if cfg!(windows) {
        let mut c = std::process::Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    } else {
        std::process::Command::new("xdg-open")
    };
    cmd.arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("cannot open the browser: {e}"))
}

/// Full PKCE login through the system browser. Blocks up to five minutes.
pub fn link(app: &tauri::AppHandle, client_id: &str) -> Result<Tokens, String> {
    CANCEL_LINK.store(false, Ordering::SeqCst);
    let listener = TcpListener::bind(("127.0.0.1", LISTEN_PORT))
        .map_err(|e| format!("port {LISTEN_PORT} is busy ({e}); close other Tone3000 sign-ins"))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let redirect = format!("http://127.0.0.1:{LISTEN_PORT}/cb");
    let verifier = b64url(&random(48));
    let challenge = b64url(&Sha256::digest(verifier.as_bytes()));
    let state = b64url(&random(16));
    let q = [
        ("client_id", client_id),
        ("redirect_uri", redirect.as_str()),
        ("response_type", "code"),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("state", state.as_str()),
        ("format", "nam"),
    ]
    .iter()
    .map(|(k, v)| format!("{k}={}", urlencoding::encode(v)))
    .collect::<Vec<_>>()
    .join("&");
    let url = format!("{API}/oauth/authorize?{q}");
    open_browser(&url)?;
    *LINK_URL.lock().unwrap_or_else(|e| e.into_inner()) = Some(url);
    let result = wait_for_callback(&listener, &state, &verifier, &redirect, client_id);
    *LINK_URL.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let tok = result?;
    save_tokens(app, &tok)?;
    Ok(tok)
}

/// Re-open the sign-in page of the link in progress.
pub fn open_link_again() -> Result<(), String> {
    let url = LINK_URL
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .ok_or("No sign-in in progress")?;
    open_browser(&url)
}

fn wait_for_callback(
    listener: &TcpListener,
    state: &str,
    verifier: &str,
    redirect: &str,
    client_id: &str,
) -> Result<Tokens, String> {
    let deadline = Instant::now() + Duration::from_secs(300);
    let params = loop {
        if CANCEL_LINK.load(Ordering::SeqCst) {
            return Err("Sign-in cancelled".into());
        }
        if Instant::now() > deadline {
            return Err("Timed out waiting for the Tone3000 sign-in".into());
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let mut line = String::new();
                let _ = BufReader::new(&mut stream).read_line(&mut line);
                let target = line.split_whitespace().nth(1).unwrap_or("").to_string();
                let (path, query) = target.split_once('?').unwrap_or((&target, ""));
                if path != "/cb" && path != "/callback" {
                    let _ =
                        stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
                    continue;
                }
                let body = "<!doctype html><meta name=viewport content='width=device-width'>\
                    <body style='font-family:system-ui;text-align:center;padding:3em'>\
                    <h2>Tone3000 linked</h2><p>You can return to TMP NAM.</p>";
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                break parse_query(query);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(150));
            }
            Err(e) => return Err(format!("callback listener failed: {e}")),
        }
    };
    let get = |k: &str| params.iter().find(|(a, _)| a == k).map(|(_, b)| b.clone());
    if let Some(err) = get("error") {
        return Err(format!("Tone3000 refused the sign-in: {err}"));
    }
    if get("state").as_deref() != Some(state) {
        return Err("Sign-in state mismatch; try again".into());
    }
    let code = get("code").ok_or("No authorization code in the callback")?;
    token_request(&[
        ("grant_type", "authorization_code"),
        ("code", &code),
        ("code_verifier", verifier),
        ("redirect_uri", redirect),
        ("client_id", client_id),
    ])
}

/// Hosts that may receive the user's Tone3000 access token.
pub fn is_tone3000_host(host: Option<&str>) -> bool {
    matches!(host, Some(h) if h == "tone3000.com" || h.ends_with(".tone3000.com"))
}

fn parse_query(q: &str) -> Vec<(String, String)> {
    q.split('&')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| {
            let dec = |s: &str| {
                urlencoding::decode(&s.replace('+', " "))
                    .map(|c| c.into_owned())
                    .unwrap_or_default()
            };
            (dec(k), dec(v))
        })
        .collect()
}

/// Authenticated session that refreshes the access token once on a 401.
pub struct Session<'a> {
    app: &'a tauri::AppHandle,
    client_id: String,
    tok: Tokens,
}

impl<'a> Session<'a> {
    pub fn new(app: &'a tauri::AppHandle, client_id: &str) -> Result<Self, String> {
        let tok = load_tokens(app).ok_or("Not signed in to Tone3000")?;
        Ok(Session {
            app,
            client_id: client_id.to_string(),
            tok,
        })
    }

    fn refresh(&mut self) -> Result<(), String> {
        let rt = self
            .tok
            .refresh_token
            .clone()
            .ok_or("Tone3000 session expired; sign in again")?;
        let mut fresh = token_request(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", &rt),
            ("client_id", &self.client_id),
        ])
        .map_err(|_| "Tone3000 session expired; sign in again".to_string())?;
        if fresh.refresh_token.is_none() {
            fresh.refresh_token = Some(rt);
        }
        save_tokens(self.app, &fresh)?;
        self.tok = fresh;
        Ok(())
    }

    fn send(&mut self, url: &str) -> Result<ureq::Response, String> {
        for attempt in 0..2 {
            let r = ureq::get(url)
                .timeout(Duration::from_secs(120))
                .set(
                    "Authorization",
                    &format!("Bearer {}", self.tok.access_token),
                )
                .call();
            match r {
                Ok(resp) => return Ok(resp),
                Err(ureq::Error::Status(401, _)) if attempt == 0 => self.refresh()?,
                Err(e) => return Err(http_err(e)),
            }
        }
        Err("Tone3000 session expired; sign in again".into())
    }

    pub fn get(&mut self, path: &str) -> Result<Value, String> {
        self.send(&format!("{API}{path}"))?
            .into_json()
            .map_err(|e| e.to_string())
    }

    /// Download a model file. The URL comes from the webview, so the bearer token is
    /// attached only to HTTPS Tone3000 hosts, and redirects are followed by hand so a
    /// hop to any other host (e.g. signed storage) is fetched without it.
    pub fn download(&mut self, url: &str) -> Result<Vec<u8>, String> {
        let agent = ureq::AgentBuilder::new()
            .redirects(0)
            .timeout(Duration::from_secs(120))
            .build();
        let mut current = url::Url::parse(url).map_err(|e| format!("bad model URL: {e}"))?;
        let mut refreshed = false;
        for _ in 0..6 {
            if current.scheme() != "https" {
                return Err(format!("refusing non-HTTPS model URL: {current}"));
            }
            let mut req = agent.get(current.as_str());
            if is_tone3000_host(current.host_str()) {
                req = req.set(
                    "Authorization",
                    &format!("Bearer {}", self.tok.access_token),
                );
            }
            let resp = match req.call() {
                Ok(r) => r,
                Err(ureq::Error::Status(401, _))
                    if !refreshed && is_tone3000_host(current.host_str()) =>
                {
                    self.refresh()?;
                    refreshed = true;
                    continue;
                }
                Err(ureq::Error::Status(code, r)) if (300..400).contains(&code) => r,
                Err(e) => return Err(http_err(e)),
            };
            if (300..400).contains(&resp.status()) {
                let location = resp
                    .header("Location")
                    .ok_or("redirect without a Location header")?
                    .to_string();
                current = current
                    .join(&location)
                    .map_err(|e| format!("bad redirect: {e}"))?;
                continue;
            }
            let mut buf = Vec::new();
            resp.into_reader()
                .take(MAX_MODEL_BYTES + 1)
                .read_to_end(&mut buf)
                .map_err(|e| format!("download failed: {e}"))?;
            if buf.len() as u64 > MAX_MODEL_BYTES {
                return Err("model is larger than 64 MiB".into());
            }
            return Ok(buf);
        }
        Err("too many redirects".into())
    }

    pub fn username(&mut self) -> Option<String> {
        let me = self.get("/user").ok()?;
        me.get("username")
            .or_else(|| me.pointer("/user/username"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct T3kModel {
    pub id: Value,
    pub name: Option<String>,
    pub size: Option<String>,
    pub architecture_version: String,
    pub variant: Option<&'static str>,
    pub model_url: Option<String>,
    pub ir_name: String,
    /// Size from `size`, or from a trailing size word in the name ("… - Lite");
    /// tells apart several A2 models of one capture.
    pub size_hint: Option<String>,
}

/// Size words uploaders append to model names, longest first; `nano-relu` is Nano.
const NAME_SIZES: [(&str, &str); 6] = [
    ("nano-relu", "nano"),
    ("standard", "standard"),
    ("feather", "feather"),
    ("lite", "lite"),
    ("nano", "nano"),
    ("full", "full"),
];

const NAME_SEPARATORS: [char; 6] = [' ', '-', '_', '(', '|', '.'];

/// Split "Amp X - Lite" into ("Amp X", Some("lite")). The size word must end the
/// name and follow a separator, so "Piano" or "Nanotube" are left alone.
pub fn split_size_suffix(name: &str) -> (&str, Option<&'static str>) {
    let trimmed = name.trim();
    let core = trimmed.strip_suffix(')').unwrap_or(trimmed).trim_end();
    let lower = core.to_ascii_lowercase();
    for (word, size) in NAME_SIZES {
        if let Some(head) = lower.strip_suffix(word) {
            if head.ends_with(NAME_SEPARATORS) {
                let base = core[..head.len()].trim_end_matches(NAME_SEPARATORS);
                if !base.is_empty() {
                    return (base, Some(size));
                }
            }
        }
    }
    (trimmed, None)
}

#[derive(Serialize, Clone, Debug)]
pub struct T3kTone {
    pub id: Value,
    pub title: String,
    pub author: Option<String>,
    pub source: &'static str,
    pub models: Vec<T3kModel>,
    /// The tone's captures: models grouped by name. Tone3000 lists one capture as
    /// an A2 model (no size) and/or A1 models (one per size) under the same name.
    /// The UI picks one model per capture.
    pub captures: Vec<T3kCapture>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct T3kCapture {
    pub name: String,
    /// Indices into the tone's `models`.
    pub models: Vec<usize>,
}

/// One entry of `/models`, classified.
fn parse_model(title: &str, m: &Value) -> T3kModel {
    let arch = str_of(m, "architecture_version").unwrap_or_default();
    let size = str_of(m, "size");
    let name = str_of(m, "name");
    let size_hint = size.as_deref().map(str::to_ascii_lowercase).or_else(|| {
        name.as_deref()
            .and_then(|n| split_size_suffix(n).1)
            .map(str::to_string)
    });
    T3kModel {
        id: m.get("id").cloned().unwrap_or(Value::Null),
        size_hint,
        variant: variants::classify(&arch, size.as_deref()),
        ir_name: variants::safe_name(Some(title), name.as_deref(), size.as_deref()),
        model_url: str_of(m, "model_url"),
        name,
        size,
        architecture_version: arch,
    }
}

/// Group a tone's models into captures by name without its size word (trimmed,
/// case-insensitive), in first-seen order; an unnamed model is its own capture,
/// named after the tone.
pub fn group_captures(title: &str, models: &[T3kModel]) -> Vec<T3kCapture> {
    let mut out: Vec<T3kCapture> = vec![];
    for (i, m) in models.iter().enumerate() {
        let name = m
            .name
            .as_deref()
            .map(|n| split_size_suffix(n).0)
            .filter(|n| !n.is_empty());
        let existing = name.and_then(|n| out.iter_mut().find(|c| c.name.eq_ignore_ascii_case(n)));
        match existing {
            Some(c) => c.models.push(i),
            None => out.push(T3kCapture {
                name: name.unwrap_or(title).to_string(),
                models: vec![i],
            }),
        }
    }
    out
}

fn str_of(v: &Value, k: &str) -> Option<String> {
    match v.get(k)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// The models list returns A1 + custom unless `architecture` is given, and never
/// A2 by default (tone3000.com/api), so each NAM architecture is asked for.
const MODEL_ARCHITECTURES: [&str; 2] = ["2", "1"];

fn models_path(tone_id: &str, architecture: &str) -> String {
    format!(
        "/models?tone_id={}&architecture={architecture}&page_size=50",
        urlencoding::encode(tone_id)
    )
}

/// Favorited ("Bookmarks") + created tones, then each tone's models, classified.
pub fn collect(session: &mut Session) -> Result<Vec<T3kTone>, String> {
    let mut tones: Vec<(Value, &'static str)> = vec![];
    let mut seen = std::collections::HashSet::new();
    for (path, source) in [
        ("/tones/favorited", "bookmark"),
        ("/tones/created", "created"),
    ] {
        for page in 1..20 {
            let payload = session.get(&format!("{path}?page={page}&page_size=25"))?;
            let rows = payload
                .get("data")
                .and_then(|d| d.as_array())
                .cloned()
                .unwrap_or_default();
            for t in &rows {
                let id = t.get("id").cloned().unwrap_or(Value::Null);
                if seen.insert(id.to_string()) {
                    tones.push((t.clone(), source));
                }
            }
            let total = payload
                .get("total_pages")
                .and_then(|v| v.as_u64())
                .unwrap_or(1);
            if page >= total || rows.is_empty() {
                break;
            }
        }
    }
    let mut out = vec![];
    for (tone, source) in tones {
        if tone
            .get("format")
            .and_then(|f| f.as_str())
            .is_some_and(|f| f != "nam")
        {
            continue;
        }
        let id = tone.get("id").cloned().unwrap_or(Value::Null);
        let title = str_of(&tone, "title").unwrap_or_else(|| "tone".into());
        let tone_id = str_of(&tone, "id").unwrap_or_default();
        let mut rows = vec![];
        for arch in MODEL_ARCHITECTURES {
            let payload = session.get(&models_path(&tone_id, arch))?;
            rows.extend(
                payload
                    .get("data")
                    .and_then(|d| d.as_array())
                    .cloned()
                    .or_else(|| payload.as_array().cloned())
                    .unwrap_or_default(),
            );
        }
        let models: Vec<T3kModel> = rows.iter().map(|m| parse_model(&title, m)).collect();
        let captures = group_captures(&title, &models);
        out.push(T3kTone {
            id,
            author: tone
                .pointer("/user/username")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            title,
            source,
            models,
            captures,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_only_goes_to_tone3000() {
        assert!(is_tone3000_host(Some("www.tone3000.com")));
        assert!(is_tone3000_host(Some("tone3000.com")));
        assert!(!is_tone3000_host(Some("tone3000.com.evil.example")));
        assert!(!is_tone3000_host(Some("eviltone3000.com")));
        assert!(!is_tone3000_host(None));
    }

    /// Read-only look at how the signed-in account's bookmarks group into captures
    /// (uses and refreshes the app's saved tokens; never prints them):
    /// `cargo test -p tmp-nam-companion t3k_account_a2 -- --ignored --nocapture`
    #[test]
    #[ignore = "needs a Tone3000 sign-in saved by the app"]
    fn t3k_account_a2_models() {
        let home = std::env::var("HOME").unwrap();
        let path = format!("{home}/Library/Application Support/dev.tmpnam.app/t3k_tokens.json");
        let mut tok: Tokens = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        // Refresh like `Session::refresh`, saving the result where the app reads it.
        let settings = format!("{home}/Library/Application Support/dev.tmpnam.app/settings.json");
        let client_id = serde_json::from_slice::<Value>(&std::fs::read(settings).unwrap()).unwrap()
            ["t3k_key"]
            .as_str()
            .unwrap()
            .to_string();
        let rt = tok.refresh_token.clone().expect("refresh token");
        let mut fresh = token_request(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", &rt),
            ("client_id", &client_id),
        ])
        .unwrap();
        if fresh.refresh_token.is_none() {
            fresh.refresh_token = Some(rt);
        }
        std::fs::write(&path, serde_json::to_vec(&fresh).unwrap()).unwrap();
        tok = fresh;
        let get = |p: &str| -> Value {
            ureq::get(&format!("{API}{p}"))
                .set("Authorization", &format!("Bearer {}", tok.access_token))
                .call()
                .map_err(http_err)
                .unwrap()
                .into_json()
                .unwrap()
        };
        let mut tones = vec![];
        for p in ["/tones/favorited", "/tones/created"] {
            let v = get(&format!("{p}?page=1&page_size=25"));
            tones.extend(v["data"].as_array().cloned().unwrap_or_default());
        }
        if tones.is_empty() {
            // Nothing bookmarked or published: look at public A2 tones instead.
            let v = get("/tones/search?architecture=2&format=nam&page_size=8");
            tones.extend(v["data"].as_array().cloned().unwrap_or_default());
        }
        println!("{} tones", tones.len());
        for t in tones.iter().take(12) {
            let id = str_of(t, "id").unwrap_or_default();
            let title = str_of(t, "title").unwrap_or_default();
            let mut rows = vec![];
            for arch in MODEL_ARCHITECTURES {
                let v = get(&models_path(&id, arch));
                rows.extend(v["data"].as_array().cloned().unwrap_or_default());
            }
            let models: Vec<T3kModel> = rows.iter().map(|m| parse_model(&title, m)).collect();
            println!("tone {id} {title:?}: {} models", models.len());
            for c in group_captures(&title, &models) {
                let kinds: Vec<String> = c
                    .models
                    .iter()
                    .map(|&i| {
                        let m = &models[i];
                        format!(
                            "A{}{}",
                            m.architecture_version,
                            m.size_hint
                                .as_deref()
                                .map(|h| format!(" {h}"))
                                .unwrap_or_default()
                        )
                    })
                    .collect();
                println!("  {:?}: {}", c.name, kinds.join(", "));
            }
        }
    }

    fn model(name: Option<&str>, arch: &str, size: Option<&str>) -> T3kModel {
        T3kModel {
            id: Value::Null,
            name: name.map(str::to_string),
            size: size.map(str::to_string),
            architecture_version: arch.into(),
            variant: variants::classify(arch, size),
            model_url: Some("https://www.tone3000.com/m".into()),
            ir_name: "x.nam".into(),
            size_hint: None,
        }
    }

    #[test]
    fn size_words_are_split_from_names() {
        assert_eq!(
            split_size_suffix("Deluxe - V5, Klon, SM57 - Nano-ReLU"),
            ("Deluxe - V5, Klon, SM57", Some("nano"))
        );
        assert_eq!(split_size_suffix("Amp X - Lite"), ("Amp X", Some("lite")));
        assert_eq!(
            split_size_suffix("Amp X (Feather)"),
            ("Amp X", Some("feather"))
        );
        assert_eq!(
            split_size_suffix("Amp_X_standard"),
            ("Amp_X", Some("standard"))
        );
        assert_eq!(split_size_suffix("Grand Piano"), ("Grand Piano", None));
        assert_eq!(split_size_suffix("Nano"), ("Nano", None));
        assert_eq!(
            split_size_suffix("RR AC30 NRM 57"),
            ("RR AC30 NRM 57", None)
        );
    }

    #[test]
    fn sized_names_group_into_one_capture() {
        let models = [
            model(Some("DR Klon SM57 - Nano-ReLU"), "2", None),
            model(Some("DR Klon SM57 - Standard"), "2", None),
            model(Some("DR Klon SM57 - Standard"), "1", Some("standard")),
            model(Some("DR Klon SM57 - Feather"), "1", Some("feather")),
        ];
        let caps = group_captures("Deluxe", &models);
        assert_eq!(caps.len(), 1, "{caps:?}");
        assert_eq!(caps[0].name, "DR Klon SM57");
        assert_eq!(caps[0].models, vec![0, 1, 2, 3]);
    }

    #[test]
    fn models_group_into_captures_by_name() {
        let models = [
            model(Some("AC30 TB 421"), "2", None),
            model(Some("AC30 NRM 57"), "2", None),
            model(Some("ac30 tb 421 "), "1", Some("standard")),
            model(Some("AC30 NRM 57"), "1", Some("feather")),
            model(None, "1", Some("nano")),
        ];
        let caps = group_captures("RR AC30", &models);
        assert_eq!(
            caps,
            vec![
                T3kCapture {
                    name: "AC30 TB 421".into(),
                    models: vec![0, 2]
                },
                T3kCapture {
                    name: "AC30 NRM 57".into(),
                    models: vec![1, 3]
                },
                T3kCapture {
                    name: "RR AC30".into(),
                    models: vec![4]
                },
            ]
        );
    }

    /// Install one Tone3000 capture on the unit the way `t3k_install` does:
    /// download (bearer only on Tone3000 hosts), validate, send with one engine
    /// restart, check it loaded, record it in installs.json. Leaves it installed.
    /// `TMPNAM_T3K_TONE=<tone id> TMPNAM_T3K_MODEL=<model id> cargo test -p
    /// tmp-nam-companion device_t3k_install -- --ignored --nocapture`
    #[test]
    #[ignore = "needs a Tone3000 sign-in saved by the app and a unit on USB"]
    fn device_t3k_install_one() {
        use crate::unit::{self, ConsoleUnit, NewModel, Unit};
        let tone_id = std::env::var("TMPNAM_T3K_TONE").expect("TMPNAM_T3K_TONE");
        let model_id = std::env::var("TMPNAM_T3K_MODEL").expect("TMPNAM_T3K_MODEL");
        let home = std::env::var("HOME").unwrap();
        let dir = format!("{home}/Library/Application Support/dev.tmpnam.app");
        let tok_path = format!("{dir}/t3k_tokens.json");
        let tok: Tokens = serde_json::from_slice(&std::fs::read(&tok_path).unwrap()).unwrap();
        let settings: Value =
            serde_json::from_slice(&std::fs::read(format!("{dir}/settings.json")).unwrap())
                .unwrap();
        let rt = tok.refresh_token.clone().expect("refresh token");
        let mut tok = token_request(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", &rt),
            ("client_id", settings["t3k_key"].as_str().unwrap()),
        ])
        .unwrap();
        if tok.refresh_token.is_none() {
            tok.refresh_token = Some(rt);
        }
        std::fs::write(&tok_path, serde_json::to_vec(&tok).unwrap()).unwrap();

        let get = |p: &str| -> Value {
            ureq::get(&format!("{API}{p}"))
                .set("Authorization", &format!("Bearer {}", tok.access_token))
                .call()
                .map_err(http_err)
                .unwrap()
                .into_json()
                .unwrap()
        };
        let tone = get(&format!("/tones/{tone_id}"));
        let title = str_of(&tone, "title").unwrap_or_default();
        let mut rows = vec![];
        for arch in MODEL_ARCHITECTURES {
            rows.extend(
                get(&models_path(&tone_id, arch))["data"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default(),
            );
        }
        let m = rows
            .iter()
            .map(|r| parse_model(&title, r))
            .find(|m| {
                str_of(&serde_json::json!({ "id": m.id }), "id").as_deref()
                    == Some(model_id.as_str())
            })
            .expect("model in tone");
        println!(
            "tone {title:?} model {:?} variant {:?} -> {}",
            m.name, m.variant, m.ir_name
        );

        // Download as Session::download does.
        let agent = ureq::AgentBuilder::new().redirects(0).build();
        let mut url = url::Url::parse(m.model_url.as_deref().unwrap()).unwrap();
        let bytes = loop {
            let mut req = agent.get(url.as_str());
            if is_tone3000_host(url.host_str()) {
                req = req.set("Authorization", &format!("Bearer {}", tok.access_token));
            }
            let resp = match req.call() {
                Ok(r) => r,
                Err(ureq::Error::Status(c, r)) if (300..400).contains(&c) => r,
                Err(e) => panic!("{}", http_err(e)),
            };
            if (300..400).contains(&resp.status()) {
                url = url.join(resp.header("Location").unwrap()).unwrap();
                continue;
            }
            let mut buf = vec![];
            resp.into_reader().read_to_end(&mut buf).unwrap();
            break buf;
        };
        let info = unit::validate_nam(&bytes).expect("a NAM model");
        let sha = unit::sha256_hex(&bytes);
        println!(
            "downloaded {} bytes, {:?}, {} submodels, sha {}",
            bytes.len(),
            info.architecture,
            info.submodels.len(),
            &sha[..12]
        );

        let name = unit::registry_name(&m.ir_name);
        let mut u = ConsoleUnit::open(&crate::console::find_port().expect("port")).expect("open");
        let out = u.add(
            vec![NewModel {
                name: name.clone(),
                bytes,
            }],
            &mut |_| {},
        );
        println!("outcome: {out:?}");
        assert_eq!(out.added, vec![name.clone()], "{out:?}");
        let listed = u
            .list()
            .unwrap()
            .into_iter()
            .find(|x| x.name == name)
            .expect("listed");
        assert!(listed.present && listed.registered, "{listed:?}");
        assert_eq!(listed.sha256.as_deref(), Some(sha.as_str()));
        println!("on unit: {} ({} bytes)", listed.name, listed.bytes);

        // Record it like `installs::record`, so the app shows its source.
        let rec_path = format!("{dir}/installs.json");
        let mut rec: serde_json::Map<String, Value> = std::fs::read(&rec_path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        rec.insert(
            sha,
            serde_json::json!({ "tone_id": tone["id"], "model_id": m.id, "variant": m.variant }),
        );
        std::fs::write(&rec_path, serde_json::to_vec_pretty(&rec).unwrap()).unwrap();
    }

    #[test]
    fn models_are_requested_per_architecture() {
        assert_eq!(
            models_path("12 3", "2"),
            "/models?tone_id=12%203&architecture=2&page_size=50"
        );
        assert_eq!(MODEL_ARCHITECTURES, ["2", "1"]);
    }

    #[test]
    fn parse_query_decodes() {
        let p = parse_query("code=a%2Bb&state=x+y&tone_id=12");
        assert_eq!(p[0], ("code".into(), "a+b".into()));
        assert_eq!(p[1], ("state".into(), "x y".into()));
    }
}
