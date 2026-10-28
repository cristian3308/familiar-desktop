use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND};
use windows::Win32::System::Threading::CreateMutexW;
use windows::core::{HSTRING, PCWSTR};

/// `HWND` wrapped as an integer so it can cross threads (`HWND` is `!Send`).
/// Window handles are global to the session, so this is sound as long as
/// the receiver tolerates the window having been destroyed meanwhile.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Hwnd(pub isize);

impl Hwnd {
    pub fn get(self) -> HWND {
        HWND(self.0 as *mut _)
    }

    pub fn is_null(self) -> bool {
        self.0 == 0
    }
}

impl From<HWND> for Hwnd {
    fn from(h: HWND) -> Self {
        Self(h.0 as isize)
    }
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// Reads a NUL-terminated wide string returned by Windows.
///
/// # Safety
/// `p` must be null or point to a valid NUL-terminated UTF-16 string.
pub unsafe fn pwstr_to_string(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut len = 0;
    while unsafe { *p.add(len) } != 0 {
        len += 1;
    }
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, len) })
}

/// Holds a named mutex for the lifetime of the process.
pub struct SingleInstance(HANDLE);

impl SingleInstance {
    /// Returns `None` if another instance already holds `name`.
    pub fn acquire(name: &str) -> Option<Self> {
        let name = HSTRING::from(name);
        let handle = unsafe { CreateMutexW(None, true, PCWSTR(name.as_ptr())) }.ok()?;
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe {
                let _ = CloseHandle(handle);
            }
            return None;
        }
        Some(Self(handle))
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
