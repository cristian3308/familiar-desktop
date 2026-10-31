//! Locale-aware clock strings (the user's own Windows format settings).

use windows::Win32::Globalization::{GetDateFormatEx, GetTimeFormatEx, TIME_NOSECONDS};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::core::{HSTRING, PCWSTR};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    pub year: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
    pub millis: u16,
    pub day: u16,
    pub month: u16,
    pub weekday: u16,
}

pub fn now() -> LocalTime {
    let t = unsafe { GetLocalTime() };
    LocalTime {
        year: t.wYear,
        hour: t.wHour,
        minute: t.wMinute,
        second: t.wSecond,
        millis: t.wMilliseconds,
        day: t.wDay,
        month: t.wMonth,
        weekday: t.wDayOfWeek,
    }
}

/// Milliseconds until the next wall-clock minute starts.
pub fn ms_to_next_minute() -> u64 {
    let t = now();
    60_000u64.saturating_sub(t.second as u64 * 1000 + t.millis as u64).max(1)
}

/// Short time in the user's format without seconds ("18:42" / "6:42 p. m.").
pub fn time_string() -> String {
    let mut buf = [0u16; 64];
    let n = unsafe { GetTimeFormatEx(None, TIME_NOSECONDS, None, None, Some(&mut buf)) };
    String::from_utf16_lossy(&buf[..(n.max(1) - 1) as usize])
}

/// Date with a custom picture in the user's language, e.g. "ddd d MMM".
pub fn date_string(picture: &str) -> String {
    date_string_in(None, picture)
}

/// Like [`date_string`] in a given locale ("en-US"); `None` is the user's.
pub fn date_string_in(locale: Option<&str>, picture: &str) -> String {
    let picture = HSTRING::from(picture);
    let locale = locale.map(HSTRING::from);
    let mut buf = [0u16; 64];
    let name = locale.as_ref().map_or(PCWSTR::null(), |l| PCWSTR(l.as_ptr()));
    let n = unsafe { GetDateFormatEx(name, Default::default(), None, &picture, Some(&mut buf), None) };
    String::from_utf16_lossy(&buf[..(n.max(1) - 1) as usize])
}
