//! `tmp-sdcard` — headless card builder for CI, Linux and maintainers. The desktop
//! app embeds the same library and command line. See `cli.rs` for usage.

fn main() -> std::process::ExitCode {
    tmp_sdcard::cli::main(std::env::args().skip(1).collect())
}
