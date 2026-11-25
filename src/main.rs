#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::process::ExitCode;

#[cfg(windows)]
fn main() -> ExitCode {
    apollo::ui::root::run()
}

#[cfg(not(windows))]
fn main() -> ExitCode {
    eprintln!("Apollo's native application requires Windows x64.");
    ExitCode::FAILURE
}
