//! Attaching a window to the wallpaper layer (`WorkerW`), so widgets sit
//! above the wallpaper — including Wallpaper Engine, which renders into the
//! same layer — but below desktop icons and every app window, and survive
//! Win+D.

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowExW, FindWindowW, GWL_STYLE, GetWindowLongPtrW, HWND_TOP, SMTO_NORMAL, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SendMessageTimeoutW, SetParent, SetWindowLongPtrW, SetWindowPos, WS_CHILD, WS_POPUP,
};
use windows::core::{BOOL, w};

/// Finds (spawning it if needed) the WorkerW window that hosts the wallpaper.
pub fn wallpaper_host() -> Option<HWND> {
    unsafe {
        let progman = FindWindowW(w!("Progman"), None).ok()?;
        // Undocumented but long-stable: asks Progman to create the WorkerW
        // behind the desktop icons (what wallpaper engines rely on).
        let _ = SendMessageTimeoutW(progman, 0x052C, WPARAM(0xD), LPARAM(0x1), SMTO_NORMAL, 1000, None);
        let _ = SendMessageTimeoutW(progman, 0x052C, WPARAM(0), LPARAM(0), SMTO_NORMAL, 1000, None);

        // Windows 11 24H2+: the WorkerW is a child of Progman.
        if let Ok(w) = FindWindowExW(Some(progman), None, w!("WorkerW"), None)
            && !w.is_invalid()
        {
            return Some(w);
        }

        // Older builds: the WorkerW is the top-level sibling right after the
        // window that contains SHELLDLL_DefView.
        let mut found = HWND::default();
        unsafe extern "system" fn find(hwnd: HWND, lp: LPARAM) -> BOOL {
            unsafe {
                if FindWindowExW(Some(hwnd), None, w!("SHELLDLL_DefView"), None).is_ok_and(|h| !h.is_invalid())
                    && let Ok(worker) = FindWindowExW(None, Some(hwnd), w!("WorkerW"), None)
                {
                    *(lp.0 as *mut HWND) = worker;
                    return false.into();
                }
            }
            true.into()
        }
        let _ = EnumWindows(Some(find), LPARAM(&mut found as *mut HWND as isize));
        (!found.is_invalid()).then_some(found)
    }
}

/// Re-parents `hwnd` into the wallpaper layer. Coordinates of the window
/// become relative to the host (which spans the virtual screen).
pub fn attach_to_wallpaper(hwnd: HWND) -> bool {
    let Some(host) = wallpaper_host() else { return false };
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        SetWindowLongPtrW(hwnd, GWL_STYLE, ((style & !WS_POPUP.0) | WS_CHILD.0) as isize);
        if SetParent(hwnd, Some(host)).is_err() {
            return false;
        }
        let _ = SetWindowPos(hwnd, Some(HWND_TOP), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
    true
}
