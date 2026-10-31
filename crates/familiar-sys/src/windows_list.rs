//! Enumerating, matching and switching between application windows.

use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, MAX_PATH, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow};
use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow};
use windows::Win32::UI::Shell::{
    QUERY_USER_NOTIFICATION_STATE, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GA_ROOT, GW_OWNER, GWL_EXSTYLE, GetAncestor, GetClassNameW, GetForegroundWindow, GetWindow,
    GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic,
    IsWindowVisible, IsZoomed, PostMessageW, SW_MINIMIZE, SW_RESTORE, SetForegroundWindow, ShowWindow,
    SwitchToThisWindow, WM_CLOSE, WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};
use windows::core::BOOL;

use windows::Win32::UI::WindowsAndMessaging::{GetWindowPlacement, WINDOWPLACEMENT, WPF_RESTORETOMAXIMIZED};

use crate::Hwnd;
use crate::util::from_wide;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppWindow {
    pub hwnd: Hwnd,
    pub title: String,
    pub exe_path: String,
    /// Explicit AppUserModelID, if the app set one (UWP apps always do).
    pub aumid: Option<String>,
    pub minimized: bool,
}

impl AppWindow {
    /// Key used to group windows under a dock icon.
    pub fn app_key(&self) -> String {
        self.aumid.clone().unwrap_or_else(|| self.exe_path.to_lowercase())
    }
}

pub fn is_top_level(hwnd: HWND) -> bool {
    unsafe { GetAncestor(hwnd, GA_ROOT) == hwnd }
}

fn is_cloaked(hwnd: HWND) -> bool {
    let mut cloaked = 0u32;
    unsafe { DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, 4).is_ok() && cloaked != 0 }
}

pub fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..n])
}

pub fn title(hwnd: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; len as usize + 1];
        let n = GetWindowTextW(hwnd, &mut buf) as usize;
        String::from_utf16_lossy(&buf[..n])
    }
}

pub fn process_id(hwnd: HWND) -> u32 {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    pid
}

pub fn process_path(pid: u32) -> String {
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return String::new();
        };
        let mut buf = [0u16; MAX_PATH as usize * 2];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(h);
        if ok.is_ok() { from_wide(&buf[..len as usize]) } else { String::new() }
    }
}

pub fn window_aumid(hwnd: HWND) -> Option<String> {
    unsafe {
        let store: IPropertyStore = SHGetPropertyStoreForWindow(hwnd).ok()?;
        let value = store.GetValue(&PKEY_AppUserModel_ID).ok()?;
        let p = PropVariantToStringAlloc(&value).ok()?;
        let s = p.to_string().ok();
        windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as *const _));
        s.filter(|s| !s.is_empty())
    }
}

/// Windows that belong in a dock/Alt+Tab: visible, unowned, not tool
/// windows, not cloaked (suspended UWP apps are cloaked), with a title.
pub fn is_app_window(hwnd: HWND) -> bool {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || is_cloaked(hwnd) {
            return false;
        }
        let owner = GetWindow(hwnd, GW_OWNER).unwrap_or_default();
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let app_window = ex & WS_EX_APPWINDOW.0 != 0;
        if !app_window && (!owner.is_invalid() || ex & WS_EX_TOOLWINDOW.0 != 0 || ex & WS_EX_NOACTIVATE.0 != 0) {
            return false;
        }
        if GetWindowTextLengthW(hwnd) == 0 {
            return false;
        }
    }
    !matches!(class_name(hwnd).as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd")
}

pub fn list_app_windows() -> Vec<AppWindow> {
    let mut handles: Vec<HWND> = Vec::new();
    unsafe extern "system" fn collect(hwnd: HWND, lp: LPARAM) -> BOOL {
        crate::safety::ffi_guard(false.into(), || {
            let v = unsafe { &mut *(lp.0 as *mut Vec<HWND>) };
            v.push(hwnd);
            true.into()
        })
    }
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut handles as *mut _ as isize));
    }
    let me = unsafe { GetCurrentProcessId() };
    handles
        .into_iter()
        .filter(|h| is_app_window(*h))
        .filter_map(|h| {
            let pid = process_id(h);
            if pid == me {
                return None;
            }
            let mut exe_path = process_path(pid);
            let mut aumid = window_aumid(h);
            // UWP apps live inside ApplicationFrameHost; use the hosted app.
            if exe_path.to_lowercase().ends_with("applicationframehost.exe") && aumid.is_none() {
                aumid = Some(String::new());
                exe_path.clear();
            }
            Some(AppWindow {
                hwnd: h.into(),
                title: title(h),
                exe_path,
                aumid: aumid.filter(|a| !a.is_empty()),
                minimized: unsafe { IsIconic(h).as_bool() },
            })
        })
        .collect()
}

pub fn foreground() -> Hwnd {
    unsafe { GetForegroundWindow() }.into()
}

/// Brings `hwnd` to the front, restoring it if minimised.
pub fn activate(hwnd: Hwnd) {
    let h = hwnd.get();
    unsafe {
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        if !SetForegroundWindow(h).as_bool() {
            // Foreground lock: fall back to the API Alt+Tab itself uses.
            SwitchToThisWindow(h, true);
        }
    }
}

/// Dock click semantics: front-most → minimise, otherwise → activate.
pub fn activate_or_minimize(hwnd: Hwnd) {
    if foreground() == hwnd && !unsafe { IsIconic(hwnd.get()) }.as_bool() {
        unsafe {
            let _ = ShowWindow(hwnd.get(), SW_MINIMIZE);
        }
    } else {
        activate(hwnd);
    }
}

pub fn close(hwnd: Hwnd) {
    unsafe {
        let _ = PostMessageW(Some(hwnd.get()), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
}

/// True when the foreground app is a D3D exclusive/borderless full-screen
/// game, a presentation, or simply covers its whole monitor.
/// Whether an app window (not the desktop, not Familiar) is in front, and
/// whether it is maximised.
pub fn foreground_app_state() -> (bool, bool) {
    let fg = unsafe { GetForegroundWindow() };
    if fg.is_invalid()
        || matches!(class_name(fg).as_str(), "Progman" | "WorkerW")
        || process_id(fg) == unsafe { GetCurrentProcessId() }
        || !is_app_window(fg)
    {
        return (false, false);
    }
    (true, unsafe { IsZoomed(fg) }.as_bool())
}

pub fn foreground_is_fullscreen() -> bool {
    let state: QUERY_USER_NOTIFICATION_STATE = unsafe { SHQueryUserNotificationState() }.unwrap_or_default();
    if state == QUNS_RUNNING_D3D_FULL_SCREEN || state == QUNS_PRESENTATION_MODE {
        return true;
    }
    let fg = unsafe { GetForegroundWindow() };
    if fg.is_invalid() || matches!(class_name(fg).as_str(), "Progman" | "WorkerW") {
        return false;
    }
    if process_id(fg) == unsafe { GetCurrentProcessId() } {
        return false;
    }
    // A maximised window briefly covers the monitor whenever the work area
    // grows (e.g. while the taskbar is hidden at startup); that is not a
    // full-screen app.
    if unsafe { IsZoomed(fg) }.as_bool() {
        return false;
    }
    // Only real app windows count: a live-wallpaper host (Lively, Wallpaper
    // Engine) or the hidden taskbar can take focus while covering the monitor.
    if !is_app_window(fg) {
        return false;
    }
    unsafe {
        let mut r = RECT::default();
        if GetWindowRect(fg, &mut r).is_err() {
            return false;
        }
        let mon = MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(mon, &mut mi).as_bool() {
            return false;
        }
        let m = mi.rcMonitor;
        r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom
    }
}

/// Visible bounds of a window in physical pixels (without the invisible
/// resize borders Windows 10/11 add around top-level windows).
pub fn window_rect(hwnd: Hwnd) -> Option<crate::window::Rect> {
    let h = hwnd.get();
    let mut r = RECT::default();
    let size = std::mem::size_of::<RECT>() as u32;
    let ok = unsafe { DwmGetWindowAttribute(h, DWMWA_EXTENDED_FRAME_BOUNDS, &mut r as *mut _ as *mut _, size) }.is_ok()
        || unsafe { GetWindowRect(h, &mut r) }.is_ok();
    (ok && r.right > r.left && r.bottom > r.top).then(|| crate::window::Rect::from_win(r))
}

/// Where the window is (or will be) when not minimised, in screen pixels:
/// the monitor work area if it restores maximised, else its normal rect.
pub fn restored_rect(hwnd: Hwnd) -> Option<crate::window::Rect> {
    let h = hwnd.get();
    unsafe {
        if !IsIconic(h).as_bool() {
            return window_rect(hwnd);
        }
        let mut wp = WINDOWPLACEMENT { length: std::mem::size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
        GetWindowPlacement(h, &mut wp).ok()?;
        let mon = MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(mon, &mut mi).as_bool() {
            return None;
        }
        if wp.flags.0 & WPF_RESTORETOMAXIMIZED.0 != 0 {
            return Some(crate::window::Rect::from_win(mi.rcWork));
        }
        // rcNormalPosition is in workspace coordinates (relative to the
        // work area's top-left).
        let n = wp.rcNormalPosition;
        let (ox, oy) = (mi.rcWork.left - mi.rcMonitor.left, mi.rcWork.top - mi.rcMonitor.top);
        Some(crate::window::Rect {
            x: n.left + ox + mi.rcMonitor.left,
            y: n.top + oy + mi.rcMonitor.top,
            w: n.right - n.left,
            h: n.bottom - n.top,
        })
    }
}
