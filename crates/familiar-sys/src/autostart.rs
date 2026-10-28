//! Start with Windows via `HKCU\...\Run`.

use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ, RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW,
};
use windows::core::{PCWSTR, w};

const VALUE: PCWSTR = w!("FamiliarCore");

pub fn set_enabled(enabled: bool) -> bool {
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Run"),
            Some(0),
            KEY_SET_VALUE,
            &mut key,
        ) != ERROR_SUCCESS
        {
            return false;
        }
        let ok = if enabled {
            let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
            let cmd: Vec<u16> = format!("\"{exe}\"").encode_utf16().chain([0]).collect();
            let bytes = std::slice::from_raw_parts(cmd.as_ptr() as *const u8, cmd.len() * 2);
            RegSetValueExW(key, VALUE, Some(0), REG_SZ, Some(bytes)) == ERROR_SUCCESS
        } else {
            let r = RegDeleteValueW(key, VALUE);
            r == ERROR_SUCCESS || r.0 == 2 // ERROR_FILE_NOT_FOUND: already off
        };
        let _ = RegCloseKey(key);
        ok
    }
}
