//! The unit's application channel: `FenderMessageTMS` protobufs over the Linux USB HID
//! gadget (VID/PID `0x1ED8`/`0x44`, 64-byte reports), the transport Pro Control uses.
//!
//! Only the `UserIR` family is spoken here. Adding an IR this way goes through
//! `tm-stomp-server` itself: it writes `/data/userIRs/<name>.wav`, appends the name to
//! its in-memory library, replies `UserIRAdded` and persists `userIRs.json` — no engine
//! restart. A message is capped at 65,535 bytes, so a NAM file (~300 KB) can't ride it:
//! the add carries a tiny placeholder WAV and the real bytes replace the file over the
//! console afterwards (the server reads IR files only when one is selected).
//! An add is acknowledged in ~110 ms and `userIRs.json` is persisted ~0.5 s later; a
//! remove is echoed in ~0.2 s; neither restarts the engine or the UI client
//! (`device_write_roundtrip`).
//!
//! Framing (both directions): `type 0x00 len <≤60 payload>` — type `0x33` start, `0x34`
//! continue, `0x35` final/standalone. Device-to-host reports carry one extra leading
//! zero. The server drops a connection after 0.75 s without traffic and then ignores
//! commands silently, so a thread and every pump heartbeat; it answers only after Pro
//! Control's first-connect burst, replayed as-is
//! (sequence and `batchStatus` values as measured by TMP Companion).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::console::{TMP_PID, TMP_VID};

const FRAME_START: u8 = 0x33;
const FRAME_CONTINUE: u8 = 0x34;
const FRAME_FINAL: u8 = 0x35;
const FRAME_PAYLOAD: usize = 60;
const MESSAGE_MAX: usize = 65_535;
const HEARTBEAT_EVERY: Duration = Duration::from_millis(250);

// ─── protobuf (hand-rolled: a dozen fields, no schema compiler) ─────────────────────

fn put_varint(out: &mut Vec<u8>, mut n: u64) {
    loop {
        let b = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

fn varint_field(out: &mut Vec<u8>, field: u32, value: u64) {
    put_varint(out, u64::from(field) << 3);
    put_varint(out, value);
}

fn bytes_field(out: &mut Vec<u8>, field: u32, value: &[u8]) {
    put_varint(out, (u64::from(field) << 3) | 2);
    put_varint(out, value.len() as u64);
    out.extend_from_slice(value);
}

fn nested(field: u32, inner: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    bytes_field(&mut out, field, inner);
    out
}

#[derive(Debug, Clone, PartialEq)]
enum Val {
    Varint(u64),
    Bytes(Vec<u8>),
    Fixed,
}

fn read_varint(buf: &[u8], pos: &mut usize) -> Option<u64> {
    let mut n = 0u64;
    for shift in (0..64).step_by(7) {
        let b = *buf.get(*pos)?;
        *pos += 1;
        n |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return Some(n);
        }
    }
    None
}

/// `(field, value)` pairs; stops at the first malformed field.
fn parse(buf: &[u8]) -> Vec<(u32, Val)> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < buf.len() {
        let Some(tag) = read_varint(buf, &mut pos) else {
            break;
        };
        let field = (tag >> 3) as u32;
        let val = match tag & 7 {
            0 => match read_varint(buf, &mut pos) {
                Some(v) => Val::Varint(v),
                None => break,
            },
            1 | 5 => {
                pos += if tag & 7 == 1 { 8 } else { 4 };
                Val::Fixed
            }
            2 => {
                let Some(len) = read_varint(buf, &mut pos) else {
                    break;
                };
                let Some(end) = pos.checked_add(len as usize).filter(|e| *e <= buf.len()) else {
                    break;
                };
                let v = buf[pos..end].to_vec();
                pos = end;
                Val::Bytes(v)
            }
            _ => break,
        };
        out.push((field, val));
    }
    out
}

fn get_bytes(fields: &[(u32, Val)], field: u32) -> Option<&[u8]> {
    fields.iter().find_map(|(f, v)| match v {
        Val::Bytes(b) if *f == field => Some(b.as_slice()),
        _ => None,
    })
}

fn get_varint(fields: &[(u32, Val)], field: u32) -> Option<u64> {
    fields.iter().find_map(|(f, v)| match v {
        Val::Varint(n) if *f == field => Some(*n),
        _ => None,
    })
}

// ─── FenderMessageTMS builders ──────────────────────────────────────────────────────

/// `TMS.<family>{ <kind>{ 1: value } }` plus `batchStatus` (field 10) when given.
fn request(family: u32, kind: u32, value: u64, batch: Option<u64>) -> Vec<u8> {
    let mut inner = Vec::new();
    varint_field(&mut inner, 1, value);
    let mut body = nested(family, &nested(kind, &inner));
    if let Some(b) = batch {
        varint_field(&mut body, 10, b);
    }
    body
}

const CONNECTION: u32 = 4;
const PRESET: u32 = 2;
const SETTINGS: u32 = 3;
const USER_IR: u32 = 13;

fn connection_request() -> Vec<u8> {
    request(CONNECTION, 1, 1, None)
}

/// Never carries a `batchStatus`: the server answers `ConnectionError` to one that does.
fn heartbeat() -> Vec<u8> {
    request(CONNECTION, 4, 1, None)
}

/// Pro Control's first-connect burst and the pump after each request (ms).
fn handshake() -> Vec<(Vec<u8>, u64)> {
    vec![
        (connection_request(), 200),
        (request(PRESET, 4, 1, Some(1)), 20),  // My Presets list
        (request(PRESET, 6, 1, Some(2)), 20),  // favorites
        (request(PRESET, 4, 4, Some(2)), 20),  // factory list
        (request(PRESET, 4, 3, Some(2)), 20),  // cloud list
        (request(PRESET, 41, 1, Some(2)), 20), // product profile
        (request(PRESET, 1, 1, Some(2)), 20),  // current preset info
        (request(SETTINGS, 66, 1, Some(2)), 20),
        (user_ir_list_request(Some(2)), 20),
        (request(PRESET, 2, 1, Some(3)), 300), // current preset data
    ]
}

/// `UserIRMessage.userIRListRequest{dummy}` (2).
fn user_ir_list_request(batch: Option<u64>) -> Vec<u8> {
    request(USER_IR, 2, 1, batch)
}

/// `UserIRMessage.addUserIR{name, irData}` (4). The server stores the file as
/// `<name>.wav`; the name is the picker entry.
fn add_user_ir(name: &str, data: &[u8]) -> Vec<u8> {
    let mut add = Vec::new();
    bytes_field(&mut add, 1, name.as_bytes());
    bytes_field(&mut add, 2, data);
    nested(USER_IR, &nested(4, &add))
}

/// `UserIRMessage.removeUserIR{slot}` (7); slots are 1-based list positions.
fn remove_user_ir(slot: u32) -> Vec<u8> {
    request(USER_IR, 7, u64::from(slot), None)
}

// ─── replies ────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum UserIrEvent {
    /// The library's names in slot order.
    List(Vec<String>),
    Added(String),
    AddFailed(String),
    Error(u64),
    /// Any other `UserIRMessage` member: (field, inner bytes).
    Other(u32, Vec<u8>),
}

/// The `UserIR` event in one reassembled `FenderMessageTMS`, if it is one.
fn user_ir_event(msg: &[u8]) -> Option<UserIrEvent> {
    let top = parse(msg);
    let ir = parse(get_bytes(&top, USER_IR)?);
    let text = |b: &[u8]| {
        get_bytes(&parse(b), 1)
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .unwrap_or_default()
    };
    if let Some(list) = get_bytes(&ir, 3) {
        let names = parse(list)
            .into_iter()
            .filter_map(|(f, v)| match v {
                Val::Bytes(r) if f == 2 => Some(text(&r)),
                _ => None,
            })
            .collect();
        return Some(UserIrEvent::List(names));
    }
    if let Some(b) = get_bytes(&ir, 5) {
        return Some(UserIrEvent::Added(text(b)));
    }
    if let Some(b) = get_bytes(&ir, 8) {
        return Some(UserIrEvent::AddFailed(text(b)));
    }
    if let Some(b) = get_bytes(&ir, 1) {
        return Some(UserIrEvent::Error(get_varint(&parse(b), 1).unwrap_or(0)));
    }
    ir.into_iter().next().map(|(f, v)| {
        UserIrEvent::Other(
            f,
            match v {
                Val::Bytes(b) => b,
                _ => Vec::new(),
            },
        )
    })
}

// ─── framing ────────────────────────────────────────────────────────────────────────

/// Split a message into 63-byte frames: one `0x35`, or `0x33` `0x34`… `0x35`.
fn frames(msg: &[u8]) -> Result<Vec<[u8; 63]>, String> {
    if msg.is_empty() || msg.len() > MESSAGE_MAX {
        return Err(format!("message of {} bytes cannot be framed", msg.len()));
    }
    let pieces: Vec<&[u8]> = msg.chunks(FRAME_PAYLOAD).collect();
    let last = pieces.len() - 1;
    Ok(pieces
        .iter()
        .enumerate()
        .map(|(i, piece)| {
            let kind = match i {
                _ if i == last => FRAME_FINAL,
                0 => FRAME_START,
                _ => FRAME_CONTINUE,
            };
            let mut f = [0u8; 63];
            f[0] = kind;
            f[2] = piece.len() as u8;
            f[3..3 + piece.len()].copy_from_slice(piece);
            f
        })
        .collect())
}

/// Reassembles device reports (`00 type lenHi lenLo payload…`) into messages.
#[derive(Default)]
struct Reassembler {
    open: Option<Vec<u8>>,
}

impl Reassembler {
    fn push(&mut self, report: &[u8]) -> Option<Vec<u8>> {
        if report.len() < 4 || report[0] != 0 {
            return None;
        }
        let len = (usize::from(report[2]) << 8) | usize::from(report[3]);
        let payload = &report[4..(4 + len).min(report.len())];
        match report[1] {
            FRAME_START => {
                self.open = Some(payload.to_vec());
                None
            }
            FRAME_CONTINUE => {
                if let Some(m) = self.open.as_mut() {
                    m.extend_from_slice(payload);
                }
                None
            }
            FRAME_FINAL => Some(match self.open.take() {
                Some(mut m) => {
                    m.extend_from_slice(payload);
                    m
                }
                None => payload.to_vec(),
            }),
            _ => None,
        }
    }
}

// ─── transport ──────────────────────────────────────────────────────────────────────

/// The device and when it was last written, shared with the heartbeat thread.
struct Link {
    dev: hidapi::HidDevice,
    last_sent: Instant,
}

impl Link {
    fn send(&mut self, msg: &[u8]) -> Result<(), String> {
        for f in frames(msg)? {
            let mut report = [0u8; 65]; // report ID 0 + 64 bytes
            report[1..64].copy_from_slice(&f);
            self.dev.write(&report).map_err(|e| e.to_string())?;
        }
        self.last_sent = Instant::now();
        Ok(())
    }
}

/// An open application session. On macOS hidapi seizes the device, so this fails while
/// Pro Control (or TMP Companion) holds it. A background thread heartbeats while the
/// caller is busy elsewhere (e.g. pushing bytes over the console).
pub struct HidSession {
    link: Arc<Mutex<Link>>,
    stop: Arc<AtomicBool>,
    beat: Option<std::thread::JoinHandle<()>>,
    rx: Reassembler,
}

impl HidSession {
    pub fn open() -> Result<Self, String> {
        let api = hidapi::HidApi::new().map_err(|e| e.to_string())?;
        let dev = api.open(TMP_VID as u16, TMP_PID as u16).map_err(|e| {
            format!("cannot open the unit's HID channel (is Pro Control running?): {e}")
        })?;
        let link = Arc::new(Mutex::new(Link {
            dev,
            last_sent: Instant::now(),
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let beat = {
            let (link, stop) = (Arc::clone(&link), Arc::clone(&stop));
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(50));
                    let mut l = link.lock().unwrap_or_else(|e| e.into_inner());
                    if l.last_sent.elapsed() >= HEARTBEAT_EVERY {
                        let _ = l.send(&heartbeat());
                    }
                }
            })
        };
        let mut s = HidSession {
            link,
            stop,
            beat: Some(beat),
            rx: Reassembler::default(),
        };
        s.handshake()?;
        Ok(s)
    }

    /// Pro Control's first-connect burst. A restarted engine ignores the session until
    /// it sees this again, so run it after every engine restart (re-arming the open
    /// device avoids macOS's re-open lockout).
    pub fn handshake(&mut self) -> Result<(), String> {
        for (msg, ms) in handshake() {
            self.send(&msg)?;
            self.pump(Duration::from_millis(ms))?;
        }
        // Let the handshake's streams (preset lists, ~17 KB product profile) finish.
        self.pump(Duration::from_millis(1500))?;
        Ok(())
    }

    fn send(&mut self, msg: &[u8]) -> Result<(), String> {
        self.link
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .send(msg)
    }

    /// Read for `d`; returns the `UserIR` events received, or the transport error.
    fn pump(&mut self, d: Duration) -> Result<Vec<UserIrEvent>, String> {
        let deadline = Instant::now() + d;
        let mut out = Vec::new();
        let mut buf = [0u8; 64];
        while Instant::now() < deadline {
            let n = {
                // Heartbeat here too: this loop re-takes the lock at once, which can
                // starve the heartbeat thread past the server's 0.75 s timeout.
                let mut l = self.link.lock().unwrap_or_else(|e| e.into_inner());
                if l.last_sent.elapsed() >= HEARTBEAT_EVERY {
                    l.send(&heartbeat())?;
                }
                l.dev
                    .read_timeout(&mut buf, 20)
                    .map_err(|e| format!("HID read: {e}"))?
            };
            if let Some(ev) = self.rx.push(&buf[..n]).and_then(|m| user_ir_event(&m)) {
                log::debug!("hid <- {ev:?}");
                out.push(ev);
            }
        }
        Ok(out)
    }

    /// Send `msg` and wait up to `d` for the first `UserIR` event `want` accepts.
    fn transact(
        &mut self,
        msg: &[u8],
        d: Duration,
        want: impl Fn(&UserIrEvent) -> bool,
    ) -> Result<UserIrEvent, String> {
        self.send(msg)?;
        let deadline = Instant::now() + d;
        let mut seen = Vec::new();
        while Instant::now() < deadline {
            for ev in self.pump(Duration::from_millis(100))? {
                if want(&ev) {
                    return Ok(ev);
                }
                seen.push(ev);
            }
        }
        Err(format!(
            "no matching UserIR reply in {d:?}; other UserIR events: {seen:?}"
        ))
    }

    /// The library as the server holds it (`userIRListRequest`, no batch).
    pub fn list(&mut self) -> Result<Vec<String>, String> {
        match self.transact(&user_ir_list_request(None), Duration::from_secs(5), |e| {
            matches!(e, UserIrEvent::List(_))
        })? {
            UserIrEvent::List(l) => Ok(l),
            _ => unreachable!(),
        }
    }

    /// Add `name` with `data` (≤ ~65 KB); the server appends it to the library.
    pub fn add(&mut self, name: &str, data: &[u8]) -> Result<(), String> {
        let ev = self.transact(&add_user_ir(name, data), Duration::from_secs(10), |e| {
            matches!(
                e,
                UserIrEvent::Added(_) | UserIrEvent::AddFailed(_) | UserIrEvent::Error(_)
            )
        })?;
        if ev != UserIrEvent::Added(name.to_string()) {
            return Err(format!("the unit refused {name}: {ev:?}"));
        }
        Ok(())
    }

    /// Remove the entry at 1-based `slot`; the server deletes the file unless another
    /// entry shares the name, and acknowledges by echoing the `RemoveUserIR`. Later
    /// slots shift down, so remove several in descending order.
    pub fn remove(&mut self, slot: u32) -> Result<(), String> {
        let ev = self.transact(&remove_user_ir(slot), Duration::from_secs(5), |e| {
            matches!(e, UserIrEvent::Other(7, _) | UserIrEvent::Error(_))
        })?;
        match ev {
            UserIrEvent::Other(7, _) => Ok(()),
            other => Err(format!("the unit refused to remove slot {slot}: {other:?}")),
        }
    }
}

impl Drop for HidSession {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.beat.take() {
            let _ = t.join();
        }
    }
}

/// A valid one-sample mono 16-bit 44.1 kHz WAV: what the add carries before the real
/// bytes replace the file, so an early selection still decodes.
pub fn placeholder_wav() -> Vec<u8> {
    let mut w = Vec::with_capacity(46);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&38u32.to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&1u16.to_le_bytes()); // mono
    w.extend_from_slice(&44_100u32.to_le_bytes());
    w.extend_from_slice(&88_200u32.to_le_bytes()); // byte rate
    w.extend_from_slice(&2u16.to_le_bytes()); // block align
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&2u32.to_le_bytes());
    w.extend_from_slice(&0i16.to_le_bytes());
    w
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builders_match_pro_control_bytes() {
        // Golden vectors shared with TMP Companion's captures.
        assert_eq!(connection_request(), [0x22, 0x04, 0x0a, 0x02, 0x08, 0x01]);
        assert_eq!(heartbeat(), [0x22, 0x04, 0x22, 0x02, 0x08, 0x01]);
        assert_eq!(
            user_ir_list_request(Some(2)),
            [0x6a, 0x04, 0x12, 0x02, 0x08, 0x01, 0x50, 0x02]
        );
        assert_eq!(remove_user_ir(3), [0x6a, 0x04, 0x3a, 0x02, 0x08, 0x03]);
    }

    #[test]
    fn add_frames_and_reassembles() {
        assert_eq!(
            frames(&heartbeat()).unwrap()[0][..9],
            [0x35, 0, 6, 0x22, 4, 0x22, 2, 8, 1]
        );
        let msg = add_user_ir("X.nam", &[7u8; 150]);
        let fr = frames(&msg).unwrap();
        assert_eq!(fr.len(), 3);
        assert_eq!(fr[0][0], FRAME_START);
        assert_eq!(fr.last().unwrap()[0], FRAME_FINAL);
        // The device prefixes a zero and sends a 16-bit length.
        let mut rx = Reassembler::default();
        let mut got = None;
        for f in &fr {
            let mut r = vec![0u8];
            r.push(f[0]);
            r.push(0);
            r.extend_from_slice(&f[2..]);
            got = rx.push(&r).or(got);
        }
        assert_eq!(got.unwrap(), msg);
        assert!(frames(&vec![0u8; MESSAGE_MAX + 1]).is_err());
    }

    #[test]
    fn parses_user_ir_replies() {
        let mut rec = Vec::new();
        bytes_field(&mut rec, 1, b"A.nam");
        varint_field(&mut rec, 2, 1);
        let mut list = Vec::new();
        varint_field(&mut list, 1, 1);
        bytes_field(&mut list, 2, &rec);
        let msg = nested(USER_IR, &nested(3, &list));
        assert_eq!(
            user_ir_event(&msg),
            Some(UserIrEvent::List(vec!["A.nam".into()]))
        );
        let mut added = Vec::new();
        bytes_field(&mut added, 1, b"B.nam");
        assert_eq!(
            user_ir_event(&nested(USER_IR, &nested(5, &added))),
            Some(UserIrEvent::Added("B.nam".into()))
        );
        assert_eq!(user_ir_event(&heartbeat()), None);
    }

    #[test]
    fn placeholder_is_a_wav() {
        let w = placeholder_wav();
        assert_eq!(w.len(), 46);
        assert_eq!(
            u32::from_le_bytes(w[4..8].try_into().unwrap()) as usize,
            w.len() - 8
        );
    }
}
