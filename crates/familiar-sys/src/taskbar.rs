//! Hiding and restoring the native Windows taskbar.
//!
//! Hiding the window alone keeps its work-area reservation, so the taskbar
//! is first switched to auto-hide (which releases the reservation) and then
//! hidden. The previous auto-hide state is returned so it can be restored.

use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::UI::Shell::{ABM_GETSTATE, ABM_SETSTATE, ABS_AUTOHIDE, APPBARDATA, SHAppBarMessage};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, FindWindowW, SW_HIDE, SW_SHOW, ShowWindow};
use windows::core::w;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TaskbarState {
    pub was_autohide: bool,
}

fn primary() -> Option<HWND> {
    unsafe { FindWindowW(w!("Shell_TrayWnd"), None) }.ok().filter(|h| !h.is_invalid())
}

fn all_taskbars() -> Vec<HWND> {
    let mut out: Vec<HWND> = primary().into_iter().collect();
    let mut prev: Option<HWND> = None;
    loop {
        let next = unsafe { FindWindowExW(None, prev, w!("Shell_SecondaryTrayWnd"), None) };
        match next {
            Ok(h) if !h.is_invalid() => {
                out.push(h);
                prev = Some(h);
            }
            _ => break,
        }
    }
    out
}

fn appbar_data(hwnd: HWND) -> APPBARDATA {
    APPBARDATA { cbSize: std::mem::size_of::<APPBARDATA>() as u32, hWnd: hwnd, ..Default::default() }
}

fn set_autohide(tray: HWND, on: bool) {
    let mut abd = appbar_data(tray);
    abd.lParam = LPARAM(if on { ABS_AUTOHIDE as isize } else { 0 });
    unsafe { SHAppBarMessage(ABM_SETSTATE, &mut abd) };
}

pub fn is_autohide() -> bool {
    let Some(tray) = primary() else { return false };
    let mut abd = appbar_data(tray);
    let state = unsafe { SHAppBarMessage(ABM_GETSTATE, &mut abd) } as u32;
    state & ABS_AUTOHIDE != 0
}

/// Hides every taskbar. Returns the state to pass to [`restore`].
pub fn hide() -> TaskbarState {
    let state = TaskbarState { was_autohide: is_autohide() };
    if let Some(tray) = primary()
        && !state.was_autohide
    {
        set_autohide(tray, true);
    }
    hide_windows();
    state
}

/// Re-hides the taskbar windows without touching auto-hide (Explorer shows
/// them again after Start opens or Explorer restarts).
pub fn hide_windows() {
    for h in all_taskbars() {
        unsafe {
            let _ = ShowWindow(h, SW_HIDE);
        }
    }
}

/// Shows the taskbar windows again without touching auto-hide (to reach
/// the notification area for a moment).
pub fn show_windows() {
    for h in all_taskbars() {
        unsafe {
            let _ = ShowWindow(h, SW_SHOW);
        }
    }
    if let Some(tray) = primary() {
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow(tray);
        }
    }
}

/// Screen rectangle of the primary taskbar.
pub fn primary_rect() -> Option<crate::window::Rect> {
    let tray = primary()?;
    let mut r = windows::Win32::Foundation::RECT::default();
    unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowRect(tray, &mut r) }.ok()?;
    Some(crate::window::Rect::from_win(r))
}

pub fn is_taskbar_window(hwnd: HWND) -> bool {
    all_taskbars().contains(&hwnd)
}

pub fn restore(state: TaskbarState) {
    for h in all_taskbars() {
        unsafe {
            let _ = ShowWindow(h, SW_SHOW);
        }
    }
    if let Some(tray) = primary()
        && !state.was_autohide
    {
        set_autohide(tray, false);
    }
}
