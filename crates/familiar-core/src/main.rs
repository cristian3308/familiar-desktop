//! Familiar Core — a native, low-footprint re-implementation of the
//! Familiar Desktop shell for Windows 11 (dock, top bar, Dynamic Island,
//! dot-matrix widgets, launcher and live themes).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
// Off Windows only the portable modules (and their tests) are built.
#![cfg_attr(not(windows), allow(dead_code))]

mod calc;
mod config;
mod i18n;
mod logger;

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod audio_pipe;
#[cfg(windows)]
mod controlcenter;
#[cfg(windows)]
mod dock;
#[cfg(windows)]
mod fx;
#[cfg(windows)]
mod indexer;
#[cfg(windows)]
mod island;
#[cfg(windows)]
mod launcher;
#[cfg(windows)]
mod lock;
#[cfg(all(windows, not(feature = "winit")))]
mod platform;
#[cfg(windows)]
mod screensaver;
#[cfg(windows)]
mod settings;
#[cfg(windows)]
mod topbar;
#[cfg(windows)]
mod ui;
#[cfg(windows)]
mod wallfx;
#[cfg(windows)]
mod widgets;

#[cfg(windows)]
fn main() {
    std::process::exit(app::main());
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Familiar Core is a Windows 10/11 shell; build it with a Windows target.");
    std::process::exit(1);
}
