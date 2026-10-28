//! Battery state and the active keyboard layout.

use windows::Win32::Globalization::LCIDToLocaleName;
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayout;
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

use crate::util::from_wide;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Battery {
    /// `None` on desktops without a battery.
    pub percent: Option<u8>,
    pub charging: bool,
    pub on_ac: bool,
    pub saver: bool,
}

pub fn battery() -> Battery {
    let mut s = SYSTEM_POWER_STATUS::default();
    if unsafe { GetSystemPowerStatus(&mut s) }.is_err() {
        return Battery::default();
    }
    let no_battery = s.BatteryFlag & 128 != 0 || s.BatteryFlag == 255;
    Battery {
        percent: (!no_battery && s.BatteryLifePercent <= 100).then_some(s.BatteryLifePercent),
        charging: s.BatteryFlag & 8 != 0,
        on_ac: s.ACLineStatus == 1,
        saver: s.SystemStatusFlag == 1,
    }
}

/// Two-letter language code of the foreground window's keyboard layout,
/// e.g. `"ES"` or `"EN"`.
pub fn keyboard_layout_code() -> String {
    unsafe {
        let fg = GetForegroundWindow();
        let tid = GetWindowThreadProcessId(fg, None);
        let hkl = GetKeyboardLayout(tid);
        let langid = (hkl.0 as usize & 0xFFFF) as u32;
        let mut buf = [0u16; 85];
        let n = LCIDToLocaleName(langid, Some(&mut buf), 0);
        if n == 0 {
            return String::new();
        }
        let name = from_wide(&buf);
        name.split('-').next().unwrap_or_default().to_uppercase()
    }
}
