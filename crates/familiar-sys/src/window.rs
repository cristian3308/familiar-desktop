//! Styling helpers that turn toolkit windows into shell surfaces.

use std::rc::Rc;

use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    CombineRgn, CreateRectRgn, CreateRoundRectRgn, DeleteObject, GetMonitorInfoW, HGDIOBJ, HRGN,
    MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MapWindowPoints, MonitorFromPoint, RGN_OR, SetWindowRgn,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    GA_PARENT, GWL_EXSTYLE, GetAncestor, GetDesktopWindow, GetWindowLongPtrW, HWND_BOTTOM, HWND_NOTOPMOST, HWND_TOP,
    HWND_TOPMOST, LWA_ALPHA, SW_HIDE, SW_SHOWNOACTIVATE, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_NOZORDER, SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPos, ShowWindow, WINDOWPOS,
    WM_WINDOWPOSCHANGING, WS_EX_APPWINDOW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT,
};

use crate::subclass;

#[derive(Clone, Copy, Debug, Default)]
pub struct SurfaceOptions {
    /// Keep the window out of the taskbar and Alt+Tab.
    pub tool_window: bool,
    /// Clicking does not steal focus from the app the user is working in.
    pub no_activate: bool,
    pub topmost: bool,
    /// Mouse input passes through to whatever is below.
    pub click_through: bool,
}

pub fn apply_surface(hwnd: HWND, opts: SurfaceOptions) {
    unsafe {
        let mut ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        if opts.tool_window {
            ex = (ex | WS_EX_TOOLWINDOW.0) & !WS_EX_APPWINDOW.0;
        }
        if opts.no_activate {
            ex |= WS_EX_NOACTIVATE.0;
        }
        let was_layered = ex & WS_EX_LAYERED.0 != 0;
        if opts.click_through {
            ex |= WS_EX_TRANSPARENT.0 | WS_EX_LAYERED.0;
        } else {
            ex &= !WS_EX_TRANSPARENT.0;
        }
        if !opts.topmost {
            ex &= !WS_EX_TOPMOST.0;
        }
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex as isize);
        // Only for windows we just made layered: calling this on a window
        // that draws with UpdateLayeredWindow would break its rendering.
        if opts.click_through && !was_layered {
            let _ = SetLayeredWindowAttributes(hwnd, Default::default(), 255, LWA_ALPHA);
        }
        let after = if opts.topmost { HWND_TOPMOST } else { HWND_NOTOPMOST };
        let _ =
            SetWindowPos(hwnd, Some(after), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED);
    }
}

/// Moves/resizes to a rectangle in *screen* physical pixels without
/// touching z-order or focus. Child windows (widgets parented to the
/// wallpaper layer) are converted to their parent's client coordinates.
pub fn set_bounds(hwnd: HWND, r: Rect) {
    unsafe {
        let mut origin = [POINT { x: r.x, y: r.y }];
        let parent = GetAncestor(hwnd, GA_PARENT);
        if !parent.is_invalid() && parent != GetDesktopWindow() {
            MapWindowPoints(None, Some(parent), &mut origin);
        }
        let _ = SetWindowPos(hwnd, None, origin[0].x, origin[0].y, r.w, r.h, SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

/// One piece of a window region, in window-relative physical pixels.
#[derive(Clone, Copy, Debug)]
pub struct RegionPart {
    pub rect: Rect,
    pub radius: i32,
}

/// Clips the window (painting, DWM backdrop *and* hit-testing) to the union
/// of `parts`. Clicks outside fall through to whatever is below.
pub fn set_region(hwnd: HWND, parts: &[RegionPart]) {
    // Per-pixel-alpha windows already hit-test by alpha; a region would
    // only fight with UpdateLayeredWindow.
    if is_layered(hwnd) {
        return;
    }
    unsafe {
        let mut combined: Option<HRGN> = None;
        for p in parts.iter().filter(|p| p.rect.w > 0 && p.rect.h > 0) {
            let r = p.rect;
            let rgn = if p.radius > 0 {
                CreateRoundRectRgn(r.x, r.y, r.x + r.w + 1, r.y + r.h + 1, p.radius * 2, p.radius * 2)
            } else {
                CreateRectRgn(r.x, r.y, r.x + r.w, r.y + r.h)
            };
            match combined {
                None => combined = Some(rgn),
                Some(c) => {
                    CombineRgn(Some(c), Some(c), Some(rgn), RGN_OR);
                    let _ = DeleteObject(HGDIOBJ(rgn.0));
                }
            }
        }
        // The system owns the region after SetWindowRgn succeeds.
        let _ = SetWindowRgn(hwnd, combined, true);
    }
}

pub fn clear_region(hwnd: HWND) {
    unsafe {
        let _ = SetWindowRgn(hwnd, None, true);
    }
}

pub fn raise_topmost(hwnd: HWND) {
    unsafe {
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

pub fn is_layered(hwnd: HWND) -> bool {
    unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_LAYERED.0 != 0 }
}

/// Re-asserts the top of the z-order for windows that are topmost.
pub fn raise_topmost_if_topmost(hwnd: HWND) {
    let ex = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
    if ex & WS_EX_TOPMOST.0 != 0 {
        raise_topmost(hwnd);
    }
}

pub fn show_no_activate(hwnd: HWND, visible: bool) {
    unsafe {
        let _ = ShowWindow(hwnd, if visible { SW_SHOWNOACTIVATE } else { SW_HIDE });
    }
}

/// Keeps a window at the bottom of the z-order (just above the desktop),
/// used for widgets when they are not attached to the wallpaper layer.
/// While the desktop itself is in front (Win+D, or a click on the
/// wallpaper), pinned windows float to the top instead: "show desktop"
/// raises the desktop window above everything at the bottom of the z-order.
static PINNED_FLOAT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn pinned_slot() -> HWND {
    if PINNED_FLOAT.load(std::sync::atomic::Ordering::Relaxed) { HWND_TOP } else { HWND_BOTTOM }
}

pub fn pin_to_bottom(hwnd: HWND) {
    subclass::add(
        hwnd,
        Rc::new(|_, msg, _, lp: LPARAM| {
            if msg == WM_WINDOWPOSCHANGING {
                // SAFETY: lParam of WM_WINDOWPOSCHANGING is a WINDOWPOS*.
                let pos = unsafe { &mut *(lp.0 as *mut WINDOWPOS) };
                pos.hwndInsertAfter = pinned_slot();
                pos.flags &= !SWP_NOZORDER;
            }
            None
        }),
    );
    restack_pinned(hwnd);
}

/// Puts a normal (non-topmost) window above every other normal window.
/// Windows refuses `SetForegroundWindow` from a background process, so a
/// window opened from a click on a no-activate surface (the top bar) would
/// otherwise appear *behind* the maximised app.
pub fn bring_to_front(hwnd: HWND) {
    let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
    unsafe {
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, flags);
        let _ = SetWindowPos(hwnd, Some(HWND_NOTOPMOST), 0, 0, 0, 0, flags);
    }
}

/// Switches pinned windows between "bottom" and "above the shown desktop".
pub fn set_pinned_float(float: bool) {
    PINNED_FLOAT.store(float, std::sync::atomic::Ordering::Relaxed);
}

pub fn restack_pinned(hwnd: HWND) {
    unsafe {
        let _ = SetWindowPos(hwnd, Some(pinned_slot()), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn from_win(r: RECT) -> Self {
        Self { x: r.left, y: r.top, w: r.right - r.left, h: r.bottom - r.top }
    }

    pub fn to_win(self) -> RECT {
        RECT { left: self.x, top: self.y, right: self.x + self.w, bottom: self.y + self.h }
    }

    pub fn contains(self, other: Rect) -> bool {
        other.x <= self.x
            && other.y <= self.y
            && other.x + other.w >= self.x + self.w
            && other.y + other.h >= self.y + self.h
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MonitorInfo {
    /// Full monitor rectangle in physical pixels.
    pub bounds: Rect,
    /// Work area (minus appbars, including ours once registered).
    pub work: Rect,
    /// 1.0 at 96 DPI.
    pub scale: f32,
}

pub fn primary_monitor() -> MonitorInfo {
    unsafe {
        let mon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut mi);
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        MonitorInfo { bounds: Rect::from_win(mi.rcMonitor), work: Rect::from_win(mi.rcWork), scale: dx as f32 / 96.0 }
    }
}
