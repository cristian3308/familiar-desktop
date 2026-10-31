//! Hiding a newly shown window before it is drawn, so its open animation
//! does not start after the window already flashed on screen.
//!
//! The shell watcher calls [`hide`] straight from its WinEvent callback (on
//! its own thread, milliseconds after `ShowWindow`), which makes the window
//! layered with alpha 0. The open animation then [`adopt`]s it and puts the
//! style back when it lands. Whatever happens, a window never stays
//! invisible: [`release`] restores it if no animation adopted it, and a
//! watchdog does so after a short timeout.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use windows::Win32::Foundation::{COLORREF, HWND};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetWindowLongPtrW, IsWindow, LWA_ALPHA, SetLayeredWindowAttributes, SetWindowLongPtrW, WS_EX_LAYERED,
};

use crate::Hwnd;

static ENABLED: AtomicBool = AtomicBool::new(false);
/// Hidden windows and their original extended style.
static HIDDEN: Mutex<Option<HashMap<isize, isize>>> = Mutex::new(None);

/// Longest a window may stay hidden without an animation taking it over.
const WATCHDOG: Duration = Duration::from_millis(1200);

/// Turned on while open animations are enabled.
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::SeqCst);
}

fn with_map<R>(f: impl FnOnce(&mut HashMap<isize, isize>) -> R) -> R {
    let mut guard = HIDDEN.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(HashMap::new))
}

/// Makes `hwnd` transparent right away. Windows that are already layered
/// manage their own alpha and are left alone.
pub fn hide(hwnd: Hwnd) {
    if !ENABLED.load(Ordering::SeqCst) {
        return;
    }
    let h = hwnd.get();
    let ex = unsafe { GetWindowLongPtrW(h, GWL_EXSTYLE) };
    if ex & WS_EX_LAYERED.0 as isize != 0 {
        return;
    }
    let fresh = with_map(|m| m.insert(hwnd.0, ex).is_none());
    if !fresh {
        return;
    }
    unsafe {
        SetWindowLongPtrW(h, GWL_EXSTYLE, ex | WS_EX_LAYERED.0 as isize);
        let _ = SetLayeredWindowAttributes(h, COLORREF(0), 0, LWA_ALPHA);
    }
    let _ = std::thread::Builder::new().name("prehide-watchdog".into()).spawn(move || {
        std::thread::sleep(WATCHDOG);
        release(hwnd);
    });
}

/// The animation takes over: returns the original extended style (the
/// caller restores it when done), or `None` if the window was not hidden.
pub fn adopt(hwnd: Hwnd) -> Option<isize> {
    with_map(|m| m.remove(&hwnd.0))
}

/// Shows the window again if it is still hidden and nobody adopted it.
pub fn release(hwnd: Hwnd) {
    if let Some(ex) = with_map(|m| m.remove(&hwnd.0)) {
        restore(hwnd.get(), ex);
    }
}

/// Shows every hidden window again (shutdown).
pub fn release_all() {
    let all: Vec<(isize, isize)> = with_map(|m| m.drain().collect());
    for (h, ex) in all {
        restore(HWND(h as *mut _), ex);
    }
}

fn restore(h: HWND, ex: isize) {
    unsafe {
        if IsWindow(Some(h)).as_bool() {
            SetWindowLongPtrW(h, GWL_EXSTYLE, ex);
        }
    }
}
