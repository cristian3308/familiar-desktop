//! Custom cursor schemes (`SetSystemCursor`) and restoring the user's own.

use std::path::Path;

use windows::Win32::UI::WindowsAndMessaging::{
    LoadCursorFromFileW, SPI_SETCURSORS, SYSTEM_CURSOR_ID, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetSystemCursor,
    SystemParametersInfoW,
};
use windows::core::{HSTRING, PCWSTR};

/// File stem -> OCR_* id.
const CURSORS: &[(&str, u32)] = &[
    ("arrow", 32512),
    ("ibeam", 32513),
    ("wait", 32514),
    ("cross", 32515),
    ("up", 32516),
    ("size_nwse", 32642),
    ("size_nesw", 32643),
    ("size_we", 32644),
    ("size_ns", 32645),
    ("size_all", 32646),
    ("no", 32648),
    ("hand", 32649),
    ("app_starting", 32650),
    ("help", 32651),
];

/// Applies every `<stem>.cur` / `<stem>.ani` found in `dir`. Returns how many
/// cursors were replaced.
pub fn apply_scheme(dir: &Path) -> usize {
    let mut applied = 0;
    for (stem, id) in CURSORS {
        let file = ["cur", "ani"].iter().map(|ext| dir.join(format!("{stem}.{ext}"))).find(|p| p.exists());
        let Some(file) = file else { continue };
        let path = HSTRING::from(file.as_os_str());
        unsafe {
            if let Ok(cursor) = LoadCursorFromFileW(PCWSTR(path.as_ptr())) {
                // SetSystemCursor takes ownership of the handle.
                if SetSystemCursor(cursor, SYSTEM_CURSOR_ID(*id)).is_ok() {
                    applied += 1;
                }
            }
        }
    }
    applied
}

/// Reloads the cursors configured in Control Panel.
pub fn restore_system() {
    unsafe {
        let _ = SystemParametersInfoW(SPI_SETCURSORS, 0, None, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
    }
}
