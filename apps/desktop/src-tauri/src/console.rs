//! USB root console transport: the SD card's CDC-ACM shell on the stock USB-C cable.
//!
//! The card runs `/bin/sh -i < /dev/ttyGS0` (BusyBox ash). On the host it shows up as
//! `/dev/cu.usbmodem*` (macOS) or `/dev/ttyACM*` (Linux). Commands are framed with
//! per-call markers so the reply is found regardless of prompt or echo noise:
//!
//! ```text
//! printf '__B_%s\n' <id>; { <cmd> ; } 2>&1; printf '\n__E_%s %d\n' <id> $?
//! ```
//!
//! The format strings never contain the expanded marker, so an echoed command line
//! can't be mistaken for the reply. `stty -echo` also flips ash's line editor into
//! its "dumb" fgets mode, which reads at most ~1 KiB per line — every line sent here
//! (heredoc bodies included) is kept well below that.

use std::ffi::CString;
use std::io;
use std::time::{Duration, Instant};

use base64::Engine;

/// Maximum bytes per line pushed through the shell (ash's dumb-mode fgets buffer is 1024).
const LINE_MAX: usize = 600;

/// The Tone Master Pro's USB identity — the same VID/PID TMP Companion matches for HID.
pub const TMP_VID: u64 = 0x1ED8;
pub const TMP_PID: u64 = 0x44;

/// The unit's console port: the CDC-ACM tty that hangs off the USB device with the
/// TMP's VID/PID. Other USB-serial gadgets are never touched.
pub fn find_port() -> Option<String> {
    if cfg!(target_os = "macos") {
        let out = std::process::Command::new("/usr/sbin/ioreg")
            .args(["-a", "-r", "-c", "IOUSBHostDevice", "-l"])
            .output()
            .ok()?;
        let tree: plist::Value = plist::from_bytes(&out.stdout).ok()?;
        tmp_callout_device(&tree)
    } else {
        linux_tmp_tty()
    }
}

/// Walk an `ioreg -a` tree; return the first `IOCalloutDevice` whose nearest USB
/// device ancestor is the TMP.
pub fn tmp_callout_device(tree: &plist::Value) -> Option<String> {
    fn walk(node: &plist::Value, ids: Option<(u64, u64)>) -> Option<String> {
        if let Some(arr) = node.as_array() {
            return arr.iter().find_map(|n| walk(n, ids));
        }
        let dict = node.as_dictionary()?;
        let num = |k: &str| dict.get(k).and_then(|v| v.as_unsigned_integer());
        let ids = match (num("idVendor"), num("idProduct")) {
            (Some(v), Some(p)) => Some((v, p)),
            _ => ids,
        };
        if ids == Some((TMP_VID, TMP_PID)) {
            if let Some(dev) = dict.get("IOCalloutDevice").and_then(|v| v.as_string()) {
                return Some(dev.to_string());
            }
        }
        dict.get("IORegistryEntryChildren")
            .and_then(|c| walk(c, ids))
    }
    walk(tree, None)
}

/// Linux: `/sys/class/tty/ttyACM*/device` is the USB interface; its parent carries
/// `idVendor`/`idProduct`.
fn linux_tmp_tty() -> Option<String> {
    let mut names: Vec<String> = std::fs::read_dir("/sys/class/tty")
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("ttyACM"))
        .collect();
    names.sort();
    names.into_iter().find_map(|n| {
        let usb = std::fs::canonicalize(format!("/sys/class/tty/{n}/device/..")).ok()?;
        let read = |f: &str| {
            std::fs::read_to_string(usb.join(f))
                .ok()
                .and_then(|s| u64::from_str_radix(s.trim(), 16).ok())
        };
        (read("idVendor")? == TMP_VID && read("idProduct")? == TMP_PID).then(|| format!("/dev/{n}"))
    })
}

/// Single-quote a value for the device shell.
pub fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub struct Console {
    fd: i32,
    pub port: String,
    pending: Vec<u8>,
    counter: u64,
}

impl Drop for Console {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.fd);
        }
    }
}

impl Console {
    pub fn open(port: &str) -> Result<Self, String> {
        let path = CString::new(port).map_err(|e| e.to_string())?;
        let fd = unsafe {
            libc::open(
                path.as_ptr(),
                libc::O_RDWR | libc::O_NOCTTY | libc::O_NONBLOCK,
            )
        };
        if fd < 0 {
            return Err(format!(
                "cannot open {port}: {} (close screen/serial tools that hold the port)",
                io::Error::last_os_error()
            ));
        }
        unsafe {
            // Exclusive: a second host reader would steal half of every reply.
            libc::ioctl(fd, libc::TIOCEXCL as _);
            let mut tio: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(fd, &mut tio) != 0 {
                libc::close(fd);
                return Err(format!("tcgetattr {port}: {}", io::Error::last_os_error()));
            }
            libc::cfmakeraw(&mut tio);
            tio.c_cflag |= libc::CLOCAL | libc::CREAD;
            libc::cfsetspeed(&mut tio, libc::B115200);
            if libc::tcsetattr(fd, libc::TCSANOW, &tio) != 0 {
                libc::close(fd);
                return Err(format!("tcsetattr {port}: {}", io::Error::last_os_error()));
            }
            libc::tcflush(fd, libc::TCIOFLUSH);
        }
        Self::from_fd(fd, port)
    }

    /// Adopt an already-open, already-configured descriptor (the tests drive a pty).
    pub fn from_fd(fd: i32, port: &str) -> Result<Self, String> {
        let mut console = Console {
            fd,
            port: port.to_string(),
            pending: Vec::new(),
            counter: 0,
        };
        console.init()?;
        Ok(console)
    }

    fn init(&mut self) -> Result<(), String> {
        // ^C abandons a half-typed line or a heredoc left open by an earlier crash.
        self.write_all(b"\x03\n")?;
        self.drain_until_quiet();
        let setup = b"stty -echo -onlcr 2>/dev/null; PS1=''; PS2=''; export PS1 PS2\n";
        // Twice: the first line may be eaten by ash's line editor (terminal queries).
        self.write_all(setup)?;
        self.drain_until_quiet();
        self.write_all(setup)?;
        self.drain_until_quiet();
        self.pending.clear();
        let (code, _) = self
            .run("true", Duration::from_secs(5))
            .map_err(|e| format!("no shell answered on {}: {e}", self.port))?;
        if code != 0 {
            return Err(format!("shell on {} did not respond cleanly", self.port));
        }
        Ok(())
    }

    fn write_all(&mut self, mut data: &[u8]) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !data.is_empty() {
            let n = unsafe { libc::write(self.fd, data.as_ptr() as *const _, data.len()) };
            if n > 0 {
                data = &data[n as usize..];
                continue;
            }
            let err = io::Error::last_os_error();
            if n < 0 && err.kind() != io::ErrorKind::WouldBlock {
                return Err(format!("console write failed: {err}"));
            }
            if Instant::now() > deadline {
                return Err("console write timed out".into());
            }
            self.poll(libc::POLLOUT, 50);
        }
        Ok(())
    }

    fn poll(&self, events: i16, ms: i32) -> bool {
        let mut pfd = libc::pollfd {
            fd: self.fd,
            events,
            revents: 0,
        };
        unsafe { libc::poll(&mut pfd, 1, ms) > 0 }
    }

    /// Read whatever arrives within `window` into `pending`.
    /// Read whatever the shell echoes, stopping once the line has been quiet for
    /// `QUIET` (at most `QUIET_MAX`). Replaces fixed 300 ms waits at connect,
    /// which made every reconnect ~0.9 s slower.
    fn drain_until_quiet(&mut self) {
        const QUIET: Duration = Duration::from_millis(120);
        const QUIET_MAX: Duration = Duration::from_millis(300);
        let start = Instant::now();
        let mut last = start;
        while start.elapsed() < QUIET_MAX && last.elapsed() < QUIET {
            match self.read_some(20) {
                Ok(n) if n > 0 => last = Instant::now(),
                Ok(_) => {}
                Err(_) => return,
            }
        }
    }

    fn read_some(&mut self, ms: i32) -> Result<usize, String> {
        if !self.poll(libc::POLLIN, ms) {
            return Ok(0);
        }
        let mut buf = [0u8; 4096];
        let n = unsafe { libc::read(self.fd, buf.as_mut_ptr() as *mut _, buf.len()) };
        if n > 0 {
            self.pending.extend_from_slice(&buf[..n as usize]);
            return Ok(n as usize);
        }
        if n == 0 {
            return Err("console closed (unit unplugged?)".into());
        }
        let err = io::Error::last_os_error();
        if err.kind() == io::ErrorKind::WouldBlock {
            Ok(0)
        } else {
            Err(format!("console read failed: {err}"))
        }
    }

    fn next_id(&mut self) -> String {
        self.counter += 1;
        let mut r = [0u8; 4];
        let _ = getrandom::fill(&mut r);
        format!("{:x}{:08x}", self.counter, u32::from_le_bytes(r))
    }

    /// Run one shell command; returns (exit code, combined stdout+stderr).
    pub fn run(&mut self, cmd: &str, timeout: Duration) -> Result<(i32, String), String> {
        let id = self.next_id();
        let line = format!(
            "printf '__B_%s\\n' {id}; {{ {cmd}\n}} 2>&1; printf '\\n__E_%s %d\\n' {id} $?\n"
        );
        self.write_all(line.as_bytes())?;
        let begin = format!("__B_{id}\n");
        let end = format!("\n__E_{id} ");
        let deadline = Instant::now() + timeout;
        loop {
            let text = String::from_utf8_lossy(&self.pending).replace('\r', "");
            if let Some(b) = text.find(&begin) {
                let body_start = b + begin.len();
                if let Some(e) = text[body_start..].find(&end) {
                    let after = &text[body_start + e + end.len()..];
                    if let Some(nl) = after.find('\n') {
                        let code: i32 = after[..nl].trim().parse().unwrap_or(-1);
                        let body = text[body_start..body_start + e].to_string();
                        self.pending.clear();
                        return Ok((code, body));
                    }
                }
            }
            if Instant::now() > deadline {
                return Err(format!("unit did not answer within {}s", timeout.as_secs()));
            }
            self.read_some(50)?;
        }
    }

    /// Run and require exit 0.
    pub fn check(&mut self, cmd: &str, timeout: Duration) -> Result<String, String> {
        let (code, out) = self.run(cmd, timeout)?;
        if code != 0 {
            return Err(format!("command failed ({code}): {}", out.trim()));
        }
        Ok(out)
    }

    /// Write `text` to `dest` on the unit through a quoted heredoc.
    pub fn write_text(&mut self, dest: &str, text: &str) -> Result<(), String> {
        let id = self.next_id();
        let tag = format!("__EOF_{id}");
        let mut cmd = format!("cat > {} <<'{tag}'\n", sh_quote(dest));
        for line in text.lines() {
            if line.len() > LINE_MAX {
                return Err("text line too long for the console".into());
            }
            cmd.push_str(line);
            cmd.push('\n');
        }
        cmd.push_str(&tag);
        self.check(&cmd, Duration::from_secs(30)).map(|_| ())
    }

    /// Stream `bytes` as base64 lines into `dest` (a .b64 file on the unit). `progress`
    /// receives the fraction sent.
    pub fn push_base64(
        &mut self,
        dest: &str,
        bytes: &[u8],
        progress: &mut dyn FnMut(f64),
    ) -> Result<(), String> {
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        let id = self.next_id();
        let tag = format!("__EOF_{id}");
        // ~2.3 KiB per acknowledged heredoc: a tty input queue (4 KiB on Linux n_tty,
        // less on some ptys) silently drops what overflows it, so never have more than
        // one small batch in flight. Each batch's exit code is the ack.
        let chunk_chars = 76 * 30;
        let total = encoded.len().max(1);
        let mut first = true;
        for (i, chunk) in encoded.as_bytes().chunks(chunk_chars).enumerate() {
            let redirect = if first { ">" } else { ">>" };
            first = false;
            let mut cmd = format!("cat {redirect} {} <<'{tag}'\n", sh_quote(dest));
            for line in chunk.chunks(76) {
                cmd.push_str(std::str::from_utf8(line).map_err(|e| e.to_string())?);
                cmd.push('\n');
            }
            cmd.push_str(&tag);
            self.check(&cmd, Duration::from_secs(120))?;
            progress(((i + 1) * chunk_chars).min(total) as f64 / total as f64);
        }
        if bytes.is_empty() {
            self.check(&format!(": > {}", sh_quote(dest)), Duration::from_secs(10))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::FromRawFd;
    use std::process::{Command, Stdio};

    /// A pty running `sh -i` stands in for the card's ttyGS0 shell.
    fn pty_shell() -> (Console, std::process::Child) {
        let (mut master, mut slave) = (0, 0);
        let rc = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(rc, 0);
        unsafe {
            let flags = libc::fcntl(master, libc::F_GETFL);
            libc::fcntl(master, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
        let io = |fd: i32| unsafe { Stdio::from_raw_fd(libc::dup(fd)) };
        let child = Command::new("/bin/bash")
            // No readline: like the card's ash after `stty -echo`, read whole lines.
            .args(["--noediting", "--norc", "-i"])
            .env("PS1", "$ ")
            .stdin(io(slave))
            .stdout(io(slave))
            .stderr(io(slave))
            .spawn()
            .unwrap();
        unsafe { libc::close(slave) };
        (Console::from_fd(master, "pty").unwrap(), child)
    }

    #[test]
    fn picks_only_the_tmp_console() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><array>
 <dict><key>idVendor</key><integer>1060</integer><key>idProduct</key><integer>9250</integer>
  <key>IORegistryEntryChildren</key><array>
   <dict><key>idVendor</key><integer>4292</integer><key>idProduct</key><integer>60000</integer>
    <key>IORegistryEntryChildren</key><array>
     <dict><key>IOCalloutDevice</key><string>/dev/cu.usbserial-OTHER</string></dict>
    </array></dict>
   <dict><key>idVendor</key><integer>7896</integer><key>idProduct</key><integer>68</integer>
    <key>IORegistryEntryChildren</key><array><dict><key>IORegistryEntryChildren</key><array>
     <dict><key>IOCalloutDevice</key><string>/dev/cu.usbmodem123454</string></dict>
    </array></dict></array></dict>
  </array></dict>
</array></plist>"#;
        let tree: plist::Value = plist::from_bytes(xml.as_bytes()).unwrap();
        assert_eq!(
            tmp_callout_device(&tree).as_deref(),
            Some("/dev/cu.usbmodem123454")
        );
    }

    #[test]
    fn frames_commands_and_exit_codes() {
        let (mut c, mut child) = pty_shell();
        let (code, out) = c
            .run("echo hello; echo err >&2", Duration::from_secs(5))
            .unwrap();
        assert_eq!(code, 0);
        assert_eq!(out.trim(), "hello\nerr");
        let (code, _) = c.run("false", Duration::from_secs(5)).unwrap();
        assert_eq!(code, 1);
        let _ = child.kill();
    }

    #[test]
    fn pushes_text_and_binary() {
        let dir = std::env::temp_dir().join(format!("tmpnam-pty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (mut c, mut child) = pty_shell();
        let text = dir.join("t.txt");
        c.write_text(text.to_str().unwrap(), "line one\nit's $HOME `x`\n")
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(&text).unwrap(),
            "line one\nit's $HOME `x`\n"
        );
        let bytes: Vec<u8> = (0..1_200_000u32).map(|i| (i * 7 % 251) as u8).collect();
        let b64 = dir.join("b.b64");
        let mut last = 0.0;
        c.push_base64(b64.to_str().unwrap(), &bytes, &mut |f| last = f)
            .unwrap();
        assert_eq!(last, 1.0);
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(std::fs::read_to_string(&b64).unwrap().replace('\n', ""))
            .unwrap();
        assert_eq!(decoded, bytes);
        let _ = child.kill();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
