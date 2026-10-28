//! Windows integration layer.
//!
//! Every long-lived watcher runs on its own thread and *blocks* in the
//! kernel (`GetMessageW`, `WaitForMultipleObjects`, `RegNotifyChangeKeyValue`)
//! so the shell costs 0% CPU while nothing changes. Nothing is injected into
//! other processes: hooks are `WINEVENT_OUTOFCONTEXT` / low-level hooks only,
//! which keeps anti-cheat and antivirus software calm.
//!
//! On non-Windows targets this crate is empty so the workspace still builds
//! and the portable crates can be tested anywhere.
#![cfg(windows)]

pub mod appbar;
pub mod apps;
pub mod audio;
pub mod autostart;
pub mod clock;
pub mod com;
pub mod cursor;
pub mod desktop;
pub mod dwm;
pub mod icons;
pub mod keyboard;
pub mod media;
pub mod mouse;
pub mod power;
pub mod privacy;
pub mod safety;
pub mod shell;
pub mod subclass;
pub mod taskbar;
pub mod util;
pub mod window;
pub mod windows_list;

pub use util::Hwnd;
