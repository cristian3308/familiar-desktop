//! DWM materials: Mica / Acrylic backdrops, rounded corners, dark mode.

use std::ffi::c_void;

use familiar_theme::BlurType;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{
    DWM_SYSTEMBACKDROP_TYPE, DWM_WINDOW_CORNER_PREFERENCE, DWMSBT_MAINWINDOW, DWMSBT_NONE, DWMSBT_TRANSIENTWINDOW,
    DWMWA_BORDER_COLOR, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_TRANSITIONS_FORCEDISABLED, DWMWA_USE_IMMERSIVE_DARK_MODE,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DWMWCP_ROUND, DwmExtendFrameIntoClientArea,
    DwmSetWindowAttribute,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::UI::Controls::MARGINS;
use windows::core::BOOL;
use windows::core::{s, w};

fn set_attr<T>(hwnd: HWND, attr: windows::Win32::Graphics::Dwm::DWMWINDOWATTRIBUTE, value: &T) -> bool {
    unsafe {
        DwmSetWindowAttribute(hwnd, attr, value as *const T as *const c_void, std::mem::size_of::<T>() as u32).is_ok()
    }
}

pub fn set_dark_mode(hwnd: HWND, dark: bool) {
    set_attr(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &BOOL::from(dark));
}

pub fn set_rounded(hwnd: HWND, rounded: bool) {
    let pref: DWM_WINDOW_CORNER_PREFERENCE = if rounded { DWMWCP_ROUND } else { DWMWCP_DONOTROUND };
    set_attr(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &pref);
}

/// Removes the 1px accent border Windows 11 draws around windows.
pub fn hide_border(hwnd: HWND) {
    const DWMWA_COLOR_NONE: u32 = 0xFFFF_FFFE;
    set_attr(hwnd, DWMWA_BORDER_COLOR, &DWMWA_COLOR_NONE);
}

/// Shell surfaces animate themselves; DWM's open/close animation would
/// double up with ours.
pub fn disable_transitions(hwnd: HWND) {
    set_attr(hwnd, DWMWA_TRANSITIONS_FORCEDISABLED, &BOOL::from(true));
}

/// Applies the theme's material. On Windows 11 22H2+ this uses the system
/// backdrop; on older builds it falls back to the (undocumented but stable)
/// `SetWindowCompositionAttribute` acrylic accent. `tint` is `0xAABBGGRR`.
pub fn set_backdrop(hwnd: HWND, blur: BlurType, tint_abgr: u32) {
    let kind: DWM_SYSTEMBACKDROP_TYPE = match blur {
        BlurType::None => DWMSBT_NONE,
        BlurType::Mica => DWMSBT_MAINWINDOW,
        BlurType::Acrylic => DWMSBT_TRANSIENTWINDOW,
    };
    if blur != BlurType::None {
        // The backdrop only shows through where the client area is
        // transparent, so extend the frame over the whole window.
        let margins = MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 };
        unsafe {
            let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);
        }
    }
    if set_attr(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &kind) {
        return;
    }
    set_accent_fallback(hwnd, blur, tint_abgr);
}

#[repr(C)]
struct AccentPolicy {
    state: u32,
    flags: u32,
    gradient: u32,
    animation_id: u32,
}

#[repr(C)]
struct CompositionData {
    attribute: u32,
    data: *mut c_void,
    size: usize,
}

type SetWindowCompositionAttribute = unsafe extern "system" fn(HWND, *mut CompositionData) -> BOOL;

fn set_accent_fallback(hwnd: HWND, blur: BlurType, tint_abgr: u32) {
    const ACCENT_DISABLED: u32 = 0;
    const ACCENT_ENABLE_BLURBEHIND: u32 = 3;
    const ACCENT_ENABLE_ACRYLICBLURBEHIND: u32 = 4;
    const WCA_ACCENT_POLICY: u32 = 19;
    let state = match blur {
        BlurType::None => ACCENT_DISABLED,
        BlurType::Mica => ACCENT_ENABLE_BLURBEHIND,
        BlurType::Acrylic => ACCENT_ENABLE_ACRYLICBLURBEHIND,
    };
    unsafe {
        let Ok(user32) = GetModuleHandleW(w!("user32.dll")) else { return };
        let Some(proc) = GetProcAddress(user32, s!("SetWindowCompositionAttribute")) else { return };
        let f: SetWindowCompositionAttribute = std::mem::transmute(proc);
        let mut policy = AccentPolicy { state, flags: 2, gradient: tint_abgr, animation_id: 0 };
        let mut data = CompositionData {
            attribute: WCA_ACCENT_POLICY,
            data: &mut policy as *mut _ as *mut c_void,
            size: std::mem::size_of::<AccentPolicy>(),
        };
        let _ = f(hwnd, &mut data);
    }
}
