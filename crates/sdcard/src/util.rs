//! Errors, hashing, host-tool lookup and subprocess helpers.

use std::fmt;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use sha2::{Digest, Sha256};

use crate::release::Pin;

pub const CHUNK_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug)]
pub struct Error(pub String);

impl Error {
    pub fn msg(s: impl Into<String>) -> Error {
        Error(s.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error(e.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn bail<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::msg(msg))
}

/// Progress sink: `progress` is a progress bar (label, done, total).
pub trait Reporter: Sync {
    fn progress(&self, label: &str, done: u64, total: u64);
    fn status(&self, message: &str);
}

/// Reporter that prints the builder's classic one-line-per-update format.
pub struct PrintReporter;

impl Reporter for PrintReporter {
    fn progress(&self, label: &str, done: u64, total: u64) {
        let ratio = if total == 0 {
            1.0
        } else {
            (done as f64 / total as f64).clamp(0.0, 1.0)
        };
        println!("{:6.2}%  {label}", ratio * 100.0);
    }
    fn status(&self, message: &str) {
        println!("{message}");
    }
}

/// Reporter emitting one JSON object per line on stdout — the protocol between an
/// elevated helper process and the GUI that tails its output.
pub struct JsonReporter;

impl Reporter for JsonReporter {
    fn progress(&self, label: &str, done: u64, total: u64) {
        println!(
            "{}",
            serde_json::json!({ "label": label, "done": done, "total": total })
        );
    }
    fn status(&self, message: &str) {
        println!("{}", serde_json::json!({ "status": message }));
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA-256 of `length` bytes at `offset` (whole file when `length` is None).
pub fn sha256_file(
    path: &Path,
    length: Option<u64>,
    offset: u64,
    report: Option<(&dyn Reporter, &str)>,
) -> Result<String> {
    let mut f =
        std::fs::File::open(path).map_err(|e| Error::msg(format!("{}: {e}", path.display())))?;
    sha256_reader(&mut f, length, offset, report)
        .map_err(|e| Error::msg(format!("{}: {e}", path.display())))
}

/// SHA-256 of `length` bytes at `offset` of an open file or device.
pub fn sha256_reader(
    f: &mut std::fs::File,
    length: Option<u64>,
    offset: u64,
    report: Option<(&dyn Reporter, &str)>,
) -> Result<String> {
    use std::io::Seek;
    let total = match length {
        Some(l) => l,
        None => f.metadata()?.len().saturating_sub(offset),
    };
    f.seek(std::io::SeekFrom::Start(offset))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK_BYTES];
    let mut remaining = length;
    let mut done = 0u64;
    let interval = 256 * 1024 * 1024;
    let mut next = interval;
    if let Some((r, label)) = report {
        r.progress(label, 0, total);
    }
    loop {
        let want = match remaining {
            Some(0) => break,
            Some(r) => (r as usize).min(CHUNK_BYTES),
            None => CHUNK_BYTES,
        };
        let n = f.read(&mut buf[..want])?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        done += n as u64;
        if let Some(r) = remaining.as_mut() {
            *r -= n as u64;
        }
        if let Some((r, label)) = report {
            if done >= next || done == total {
                r.progress(label, done, total);
                next = done + interval;
            }
        }
    }
    if matches!(remaining, Some(r) if r != 0) {
        return bail("short read");
    }
    Ok(hex(&hasher.finalize()))
}

pub fn assert_file(path: &Path, expected: &Pin) -> Result<String> {
    if !path.is_file() {
        return bail(format!("required file missing: {}", path.display()));
    }
    if std::fs::metadata(path)?.len() != expected.bytes {
        return bail(format!("unexpected size for {}", path.display()));
    }
    let digest = sha256_file(path, None, 0, None)?;
    if digest != expected.sha256 {
        return bail(format!("unexpected SHA-256 for {}", path.display()));
    }
    Ok(digest)
}

const TOOL_DIRS: &[&str] = &[
    "/opt/homebrew/opt/util-linux/sbin",
    "/usr/local/opt/util-linux/sbin",
    "/opt/homebrew/opt/util-linux/bin",
    "/usr/local/opt/util-linux/bin",
    "/opt/homebrew/opt/e2fsprogs/sbin",
    "/usr/local/opt/e2fsprogs/sbin",
    "/opt/homebrew/bin",
    "/usr/local/bin",
    "/usr/bin",
    "/bin",
    "/usr/sbin",
    "/sbin",
];

/// Find a host tool on PATH or in the Homebrew/system locations a GUI app's
/// minimal PATH misses (keg-only e2fsprogs/util-linux included).
pub fn locate_tool(name: &str) -> Result<PathBuf> {
    let path = std::env::var("PATH").unwrap_or_default();
    let dirs = path
        .split(':')
        .filter(|d| !d.is_empty())
        .chain(TOOL_DIRS.iter().copied());
    for d in dirs {
        let p = Path::new(d).join(name);
        if is_executable(&p) {
            return Ok(p);
        }
    }
    bail(format!("required tool not found: {name}"))
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

pub struct Output {
    pub stdout: String,
    pub stderr: String,
}

fn describe(cmd: &[&dyn AsRefOsStr]) -> String {
    cmd.iter()
        .map(|a| a.as_os().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

pub trait AsRefOsStr {
    fn as_os(&self) -> &std::ffi::OsStr;
}

impl<T: AsRef<std::ffi::OsStr>> AsRefOsStr for T {
    fn as_os(&self) -> &std::ffi::OsStr {
        self.as_ref()
    }
}

/// Run a command to completion; non-zero exit is an error carrying its output.
pub fn run(cmd: &[&dyn AsRefOsStr], stdin: Option<&str>) -> Result<Output> {
    let (program, args) = cmd
        .split_first()
        .ok_or_else(|| Error::msg("empty command"))?;
    let mut c = Command::new(program.as_os());
    for a in args {
        c.arg(a.as_os());
    }
    c.stdout(Stdio::piped()).stderr(Stdio::piped());
    c.stdin(if stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    let mut child = c
        .spawn()
        .map_err(|e| Error::msg(format!("cannot run {}: {e}", describe(&cmd[..1]))))?;
    if let Some(text) = stdin {
        child
            .stdin
            .take()
            .expect("piped")
            .write_all(text.as_bytes())?;
    }
    let out = child.wait_with_output()?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if !out.status.success() {
        let detail = format!("\n{stdout}{stderr}");
        return bail(format!(
            "command failed ({}): {}{}",
            out.status.code().unwrap_or(-1),
            describe(cmd),
            detail.trim_end()
        ));
    }
    Ok(Output { stdout, stderr })
}

/// `run!(a, b, c)` with heterogeneous OsStr-like arguments.
#[macro_export]
macro_rules! run {
    ($stdin:expr; $($arg:expr),+ $(,)?) => {
        $crate::util::run(&[$(&$arg as &dyn $crate::util::AsRefOsStr),+], $stdin)
    };
    ($($arg:expr),+ $(,)?) => {
        $crate::util::run(&[$(&$arg as &dyn $crate::util::AsRefOsStr),+], None)
    };
}

/// Write all bytes to a raw device that may return short writes.
pub fn write_all(f: &mut std::fs::File, mut data: &[u8]) -> Result<()> {
    while !data.is_empty() {
        let n = f.write(data)?;
        if n == 0 {
            return bail("write made no forward progress");
        }
        data = &data[n..];
    }
    Ok(())
}

pub fn format_bytes(n: u64) -> String {
    let mut v = n as f64;
    for unit in ["B", "KiB", "MiB", "GiB"] {
        if v < 1024.0 || unit == "GiB" {
            return if unit == "B" {
                format!("{n} B")
            } else {
                format!("{v:.1} {unit}")
            };
        }
        v /= 1024.0;
    }
    unreachable!()
}

/// Remove a work tree that may contain read-only directories from the firmware.
pub fn remove_tree(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fn unlock(p: &Path) {
        if let Ok(meta) = std::fs::symlink_metadata(p) {
            if meta.is_dir() {
                let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o700));
                if let Ok(rd) = std::fs::read_dir(p) {
                    for e in rd.flatten() {
                        unlock(&e.path());
                    }
                }
            }
        }
    }
    unlock(path);
    let _ = std::fs::remove_dir_all(path);
}
