# Security Policy

## Reporting a vulnerability

Please report security vulnerabilities **privately** — do not open a public issue.

Use GitHub's [private vulnerability reporting](https://github.com/pcavadas/tmp-nam/security/advisories/new)
(the repo's **Security → Report a vulnerability**) to open a confidential advisory that only the
maintainer can see.

## Scope

TMP NAM is an owner-side tool: a desktop app (macOS, Linux) that talks to a Fender Tone Master Pro
**you own**, plus the SD card it builds from your own copy of the official firmware. It has no
server and no multi-user surface.

By design, the booted card exposes an **unauthenticated root shell** on the USB console, and SSH
(Dropbear) when Wi-Fi is configured — see [docs/device/](docs/device/). That is documented
behavior, not a vulnerability; ways to reach it beyond what the docs describe are in scope.

The areas most relevant to a report:

- Parsing of untrusted input: console framing and file push (`apps/desktop/src-tauri/src/console.rs`),
  the HID channel (`hid.rs`), Tone3000 OAuth and downloads (`t3k.rs`), `.nam` files.
- The privileged path: the app re-runs its own binary as `--sdcard-helper` through an
  administrator prompt to write physical cards (`apps/desktop/src-tauri/src/sdcard.rs`,
  `crates/sdcard/src/card.rs`) — anything that could make it write to the wrong disk.
- Storage of the Tone3000 key and tokens (`settings.rs`, `t3k.rs`).
- The card's network services (`device/systemd/`, `device/scripts/`).

## What to expect

This is a solo-maintained project: responses are best-effort and there is no bug-bounty
program. Valid reports will be addressed and, if you wish, credited in the advisory and release
notes.
