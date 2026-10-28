//! Microphone / camera privacy dots.
//!
//! Windows records capability use under
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\<capability>`:
//! one subkey per packaged app plus `NonPackaged\<mangled exe path>` for
//! desktop apps. An app is using the device while its `LastUsedTimeStop`
//! is 0. `RegNotifyChangeKeyValue` lets the thread sleep until that changes.

use std::sync::Arc;
use std::thread::JoinHandle;

use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_NOTIFY, KEY_READ, REG_NOTIFY_CHANGE_LAST_SET, REG_NOTIFY_CHANGE_NAME,
    RRF_RT_REG_QWORD, RegCloseKey, RegEnumKeyExW, RegGetValueW, RegNotifyChangeKeyValue, RegOpenKeyExW,
};
use windows::Win32::System::Threading::{CreateEventW, INFINITE, SetEvent, WaitForMultipleObjects};
use windows::core::{HSTRING, PCWSTR, PWSTR, w};

const CONSENT_STORE: &str = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrivacyState {
    /// Apps currently using the microphone (display names, best effort).
    pub microphone: Vec<String>,
    pub camera: Vec<String>,
    pub location: Vec<String>,
}

impl PrivacyState {
    pub fn mic_active(&self) -> bool {
        !self.microphone.is_empty()
    }

    pub fn cam_active(&self) -> bool {
        !self.camera.is_empty()
    }
}

struct Key(HKEY);

impl Key {
    fn open(parent: HKEY, path: &str, access: windows::Win32::System::Registry::REG_SAM_FLAGS) -> Option<Self> {
        let mut out = HKEY::default();
        let path = HSTRING::from(path);
        let r = unsafe { RegOpenKeyExW(parent, PCWSTR(path.as_ptr()), Some(0), access, &mut out) };
        (r == ERROR_SUCCESS).then_some(Self(out))
    }

    fn subkeys(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut buf = [0u16; 512];
        for i in 0.. {
            let mut len = buf.len() as u32;
            let r =
                unsafe { RegEnumKeyExW(self.0, i, Some(PWSTR(buf.as_mut_ptr())), &mut len, None, None, None, None) };
            if r != ERROR_SUCCESS {
                break;
            }
            out.push(String::from_utf16_lossy(&buf[..len as usize]));
        }
        out
    }

    fn qword(&self, sub: &str, name: PCWSTR) -> Option<u64> {
        let mut v = 0u64;
        let mut size = 8u32;
        let sub = HSTRING::from(sub);
        let r = unsafe {
            RegGetValueW(
                self.0,
                PCWSTR(sub.as_ptr()),
                name,
                RRF_RT_REG_QWORD,
                None,
                Some(&mut v as *mut u64 as *mut _),
                Some(&mut size),
            )
        };
        (r == ERROR_SUCCESS).then_some(v)
    }
}

impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

fn in_use(capability: &Key) -> Vec<String> {
    let mut apps = Vec::new();
    let mut check = |key: &Key, sub: &str, label: String| {
        let start = key.qword(sub, w!("LastUsedTimeStart")).unwrap_or(0);
        let stop = key.qword(sub, w!("LastUsedTimeStop"));
        if start != 0 && stop == Some(0) {
            apps.push(label);
        }
    };
    for sub in capability.subkeys() {
        if sub == "NonPackaged" {
            if let Some(np) = Key::open(capability.0, "NonPackaged", KEY_READ) {
                for exe in np.subkeys() {
                    // Paths are stored with '#' instead of '\'.
                    let label = exe.rsplit('#').next().unwrap_or(&exe).to_string();
                    check(&np, &exe, label);
                }
            }
        } else {
            let label = sub.split('_').next().unwrap_or(&sub).to_string();
            check(capability, &sub, label);
        }
    }
    apps
}

pub fn current() -> PrivacyState {
    let read = |cap: &str| {
        Key::open(HKEY_CURRENT_USER, &format!(r"{CONSENT_STORE}\{cap}"), KEY_READ)
            .map(|k| in_use(&k))
            .unwrap_or_default()
    };
    PrivacyState { microphone: read("microphone"), camera: read("webcam"), location: read("location") }
}

pub struct PrivacyWatcher {
    stop: Arc<OwnedEvent>,
    join: Option<JoinHandle<()>>,
}

struct OwnedEvent(HANDLE);
// SAFETY: event handles may be used from any thread.
unsafe impl Send for OwnedEvent {}
unsafe impl Sync for OwnedEvent {}

impl Drop for OwnedEvent {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

impl PrivacyWatcher {
    /// Calls `on_change` with the initial state and after every change.
    pub fn spawn(on_change: impl Fn(PrivacyState) + Send + 'static) -> std::io::Result<Self> {
        let stop = unsafe { CreateEventW(None, true, false, None) }.map_err(std::io::Error::other)?;
        let stop = Arc::new(OwnedEvent(stop));
        let thread_stop = stop.clone();
        let join = std::thread::Builder::new().name("privacy-watch".into()).spawn(move || {
            let Some(root) = Key::open(HKEY_CURRENT_USER, CONSENT_STORE, KEY_NOTIFY | KEY_READ) else {
                log::warn!("privacy: ConsentStore key not found");
                return;
            };
            let Ok(changed) = (unsafe { CreateEventW(None, false, false, None) }) else { return };
            let changed = OwnedEvent(changed);
            let mut last = current();
            on_change(last.clone());
            loop {
                let armed = unsafe {
                    RegNotifyChangeKeyValue(
                        root.0,
                        true,
                        REG_NOTIFY_CHANGE_NAME | REG_NOTIFY_CHANGE_LAST_SET,
                        Some(changed.0),
                        true,
                    )
                };
                if armed != ERROR_SUCCESS {
                    log::warn!("privacy: RegNotifyChangeKeyValue failed: {armed:?}");
                    return;
                }
                let r = unsafe { WaitForMultipleObjects(&[changed.0, thread_stop.0], false, INFINITE) };
                if r != WAIT_OBJECT_0 {
                    return;
                }
                let now = current();
                if now != last {
                    last = now.clone();
                    on_change(now);
                }
            }
        })?;
        Ok(Self { stop, join: Some(join) })
    }
}

impl Drop for PrivacyWatcher {
    fn drop(&mut self) {
        unsafe {
            let _ = SetEvent(self.stop.0);
        }
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}
