// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> std::process::ExitCode {
    // Card-builder helper mode: the app re-runs itself (unprivileged for image
    // files, elevated for physical cards) instead of shipping a separate binary.
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() == Some("--sdcard-helper") {
        return tmp_sdcard::cli::main(args.collect());
    }
    tmp_nam_companion_lib::run();
    std::process::ExitCode::SUCCESS
}
