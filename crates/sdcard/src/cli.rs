//! Command line shared by the `tmp-sdcard` binary and the desktop app's elevated
//! helper mode (`TMP NAM --sdcard-helper …`).
//!
//! ```text
//! image <firmware.img> <output.img>
//! write <firmware.img> --device /dev/diskN [--yes] [--expect-bytes N --expect-model M]
//! list
//! verify
//! ```
//! `--device-dir <dir>` selects the pinned assets (default: the repository's
//! `device/`); `--json` prints one JSON object per progress line.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::util::{format_bytes, JsonReporter, PrintReporter, Reporter};
use crate::{card, DeviceDir, Error, Release, Result};

/// Exit status when the administrator prompt was cancelled (the app's "denied").
pub const DENIED: u8 = 130;
/// Exit status when macOS refused removable-volume access (the app's "blocked");
/// sysexits(3) `EX_NOPERM`.
pub const BLOCKED: u8 = 77;

const USAGE: &str = "usage:
  tmp-sdcard image <firmware.img> <output.img>
  tmp-sdcard write <firmware.img> --device /dev/diskN [--yes]
  tmp-sdcard list
  tmp-sdcard verify
options: --device-dir <dir>  --json";

pub fn main(mut args: Vec<String>) -> ExitCode {
    let mut take = |flag: &str| -> Option<String> {
        let i = args.iter().position(|a| a == flag)?;
        args.remove(i);
        (i < args.len()).then(|| args.remove(i))
    };
    let device_dir = take("--device-dir").map(PathBuf::from);
    let device = take("--device");
    let expect_bytes = take("--expect-bytes").and_then(|v| v.parse::<u64>().ok());
    let expect_model = take("--expect-model");
    let json = args.iter().any(|a| a == "--json");
    let yes = args.iter().any(|a| a == "--yes");
    args.retain(|a| a != "--json" && a != "--yes");

    let reporter: &dyn Reporter = if json { &JsonReporter } else { &PrintReporter };
    let release = Release::embedded();
    let device_dir = device_dir
        .map(DeviceDir)
        .unwrap_or_else(DeviceDir::repository);

    let result = match (args.first().map(String::as_str), args.len()) {
        (Some("image"), 3) => crate::build_image(
            &release,
            &device_dir,
            args[1].as_ref(),
            args[2].as_ref(),
            reporter,
        ),
        (Some("write"), 2) if device.is_some() => {
            let expect = expect_bytes.map(|bytes| (bytes, expect_model.unwrap_or_default()));
            write(
                &release,
                &device_dir,
                &args[1],
                &device.unwrap_or_default(),
                yes,
                expect,
                reporter,
            )
        }
        (Some("list"), 1) => card::candidates().map(|list| {
            for c in list {
                let refused = c
                    .rejected
                    .map(|r| format!("  (refused: {r})"))
                    .unwrap_or_default();
                println!(
                    "{}  {}  {}  {}{refused}",
                    c.device,
                    format_bytes(c.bytes),
                    c.protocol,
                    c.name
                );
            }
        }),
        (Some("verify"), 1) => device_dir
            .verify(&release)
            .map(|_| println!("device assets match release pins")),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            if json {
                println!("{}", serde_json::json!({ "error": e.0 }));
            } else {
                eprintln!("ERROR: {e}");
            }
            if e.0 == card::ACCESS_DENIED {
                ExitCode::from(DENIED)
            } else if e.0.starts_with(card::VOLUMES_BLOCKED) {
                ExitCode::from(BLOCKED)
            } else {
                ExitCode::FAILURE
            }
        }
    }
}

fn write(
    release: &Release,
    device_dir: &DeviceDir,
    firmware: &str,
    device: &str,
    yes: bool,
    expect: Option<(u64, String)>,
    r: &dyn Reporter,
) -> Result<()> {
    let target = card::device_info(device)?;
    card::validate_capacity(&target, release)?;
    if let Some((bytes, model)) = &expect {
        if *bytes != target.bytes || *model != target.model {
            return Err(Error::msg(
                "SD card changed since it was confirmed; nothing was written",
            ));
        }
    }
    if !yes {
        println!(
            "SD card:  {} · {} · {}",
            target.logical,
            target.model,
            format_bytes(target.bytes)
        );
        print!("This will erase the entire card. Continue? [y/N] ");
        std::io::stdout().flush()?;
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        if !matches!(line.trim().to_lowercase().as_str(), "y" | "yes") {
            return Err(Error::msg("cancelled; no write performed"));
        }
    }
    let disk = card::open(&target)?;
    let p = crate::build_rootfs(release, device_dir, firmware.as_ref(), r)?;
    card::write_prepared(device, Some(&target), disk, &p, release, r)
}
