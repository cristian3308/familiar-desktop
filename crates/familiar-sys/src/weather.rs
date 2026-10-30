//! Weather from Open-Meteo (free, no API key). This is the only network
//! access in Familiar Core and it only runs when the weather widget is on.
//!
//! HTTP goes through WinHTTP, so TLS and proxies are the system's and no
//! HTTP stack is bundled. (WinRT's `HttpClient` was tried first but fails
//! with ERROR_INTERNET_SECURE_FAILURE on machines where revocation servers
//! are unreachable; WinHTTP does not check revocation by default.)

use serde::Deserialize;
use windows::Win32::Networking::WinHttp::{
    WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_OPEN_REQUEST_FLAGS, WinHttpCloseHandle,
    WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryDataAvailable, WinHttpReadData,
    WinHttpReceiveResponse, WinHttpSendRequest,
};
use windows::core::{HSTRING, PCWSTR, w};

#[derive(Clone, Debug, PartialEq)]
pub struct Hour {
    /// Local hour of day (0–23).
    pub hour: u32,
    pub temp: f32,
    pub code: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Weather {
    pub city: String,
    pub temp: f32,
    pub code: u32,
    pub is_day: bool,
    pub high: f32,
    pub low: f32,
    pub hours: Vec<Hour>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub latitude: f64,
    pub longitude: f64,
}

/// Simplified condition for drawing, from a WMO weather code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sky {
    Clear,
    PartlyCloudy,
    Cloudy,
    Fog,
    Rain,
    Snow,
    Storm,
}

impl Sky {
    pub fn from_code(code: u32) -> Self {
        match code {
            0 => Self::Clear,
            1 | 2 => Self::PartlyCloudy,
            3 => Self::Cloudy,
            45 | 48 => Self::Fog,
            51..=67 | 80..=82 => Self::Rain,
            71..=77 | 85 | 86 => Self::Snow,
            95..=99 => Self::Storm,
            _ => Self::Cloudy,
        }
    }

    pub fn describe_es(self) -> &'static str {
        match self {
            Self::Clear => "Despejado",
            Self::PartlyCloudy => "Parcialmente nublado",
            Self::Cloudy => "Nublado",
            Self::Fog => "Niebla",
            Self::Rain => "Lluvia",
            Self::Snow => "Nieve",
            Self::Storm => "Tormenta",
        }
    }
}

#[derive(Debug)]
pub enum WeatherError {
    Http(windows::core::Error),
    Json(serde_json::Error),
    UnknownCity(String),
    Incomplete,
}

impl std::fmt::Display for WeatherError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http(e) => write!(f, "http: {e}"),
            Self::Json(e) => write!(f, "json: {e}"),
            Self::UnknownCity(c) => write!(f, "city not found: {c}"),
            Self::Incomplete => write!(f, "incomplete forecast"),
        }
    }
}

impl From<windows::core::Error> for WeatherError {
    fn from(e: windows::core::Error) -> Self {
        Self::Http(e)
    }
}

impl From<serde_json::Error> for WeatherError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

/// Closes a WinHTTP handle when dropped.
struct WinHttpHandle(*mut std::ffi::c_void);

impl Drop for WinHttpHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

fn check(h: *mut std::ffi::c_void) -> Result<WinHttpHandle, WeatherError> {
    if h.is_null() { Err(windows::core::Error::from_thread().into()) } else { Ok(WinHttpHandle(h)) }
}

/// HTTPS GET of `host` + `path_and_query`; returns the body as text.
fn get(host: &str, path_and_query: &str) -> Result<String, WeatherError> {
    unsafe {
        let session = check(WinHttpOpen(
            w!("FamiliarCore/0.1"),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ))?;
        let connect = check(WinHttpConnect(session.0, &HSTRING::from(host), 443, 0))?;
        let request = check(WinHttpOpenRequest(
            connect.0,
            w!("GET"),
            &HSTRING::from(path_and_query),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_OPEN_REQUEST_FLAGS(WINHTTP_FLAG_SECURE.0),
        ))?;
        WinHttpSendRequest(request.0, None, None, 0, 0, 0)?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut())?;
        let mut body = Vec::new();
        loop {
            let mut available = 0u32;
            WinHttpQueryDataAvailable(request.0, &mut available)?;
            if available == 0 {
                break;
            }
            let mut chunk = vec![0u8; available as usize];
            let mut read = 0u32;
            WinHttpReadData(request.0, chunk.as_mut_ptr().cast(), available, &mut read)?;
            body.extend_from_slice(&chunk[..read as usize]);
        }
        Ok(String::from_utf8_lossy(&body).into_owned())
    }
}

/// Percent-encodes a query value (UTF-8).
fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[derive(Deserialize)]
struct GeoResponse {
    #[serde(default)]
    results: Vec<GeoResult>,
}

#[derive(Deserialize)]
struct GeoResult {
    name: String,
    latitude: f64,
    longitude: f64,
}

/// Looks a city up; returns its display name and coordinates.
pub fn geocode(city: &str) -> Result<(String, Place), WeatherError> {
    let path = format!("/v1/search?name={}&count=1&language=es&format=json", encode(city.trim()));
    let r: GeoResponse = serde_json::from_str(&get("geocoding-api.open-meteo.com", &path)?)?;
    let g = r.results.into_iter().next().ok_or_else(|| WeatherError::UnknownCity(city.into()))?;
    Ok((g.name, Place { latitude: g.latitude, longitude: g.longitude }))
}

#[derive(Deserialize)]
struct Forecast {
    current: Current,
    hourly: Hourly,
    daily: Daily,
}

#[derive(Deserialize)]
struct Current {
    time: String,
    temperature_2m: f32,
    weather_code: u32,
    is_day: u8,
}

#[derive(Deserialize)]
struct Hourly {
    time: Vec<String>,
    temperature_2m: Vec<f32>,
    weather_code: Vec<u32>,
}

#[derive(Deserialize)]
struct Daily {
    temperature_2m_max: Vec<f32>,
    temperature_2m_min: Vec<f32>,
}

pub fn forecast(city: &str, place: Place) -> Result<Weather, WeatherError> {
    let path = format!(
        "/v1/forecast?latitude={:.4}&longitude={:.4}\
         &current=temperature_2m,weather_code,is_day&hourly=temperature_2m,weather_code\
         &daily=temperature_2m_max,temperature_2m_min&timezone=auto&forecast_days=2",
        place.latitude, place.longitude
    );
    parse_forecast(city, &get("api.open-meteo.com", &path)?)
}

/// Geocodes `city` and fetches its forecast. Blocking: call it from a
/// background thread.
pub fn fetch(city: &str) -> Result<Weather, WeatherError> {
    let (name, place) = geocode(city)?;
    forecast(&name, place)
}

fn parse_forecast(city: &str, json: &str) -> Result<Weather, WeatherError> {
    let f: Forecast = serde_json::from_str(json)?;
    // Hourly times look like "2026-10-29T16:00"; start at the current hour.
    let now_hour = f.current.time.get(..13).unwrap_or_default();
    let start = f.hourly.time.iter().position(|t| t.get(..13).unwrap_or_default() >= now_hour).unwrap_or(0);
    let hours = (start..f.hourly.time.len())
        .take(6)
        .filter_map(|i| {
            let hour = f.hourly.time[i].get(11..13)?.parse().ok()?;
            Some(Hour { hour, temp: *f.hourly.temperature_2m.get(i)?, code: *f.hourly.weather_code.get(i)? })
        })
        .collect();
    Ok(Weather {
        city: city.into(),
        temp: f.current.temperature_2m,
        code: f.current.weather_code,
        is_day: f.current.is_day != 0,
        high: *f.daily.temperature_2m_max.first().ok_or(WeatherError::Incomplete)?,
        low: *f.daily.temperature_2m_min.first().ok_or(WeatherError::Incomplete)?,
        hours,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_open_meteo_forecast() {
        let json = r#"{
            "current": {"time": "2026-10-29T16:15", "temperature_2m": 19.4, "weather_code": 2, "is_day": 1},
            "hourly": {
                "time": ["2026-10-29T15:00", "2026-10-29T16:00", "2026-10-29T17:00"],
                "temperature_2m": [20.0, 19.5, 18.1],
                "weather_code": [1, 2, 61]
            },
            "daily": {"temperature_2m_max": [21.3], "temperature_2m_min": [11.0]}
        }"#;
        let w = parse_forecast("Bogotá", json).unwrap();
        assert_eq!(w.hours.len(), 2);
        assert_eq!(w.hours[0], Hour { hour: 16, temp: 19.5, code: 2 });
        assert_eq!(Sky::from_code(w.hours[1].code), Sky::Rain);
        assert!(w.is_day);
        assert_eq!((w.high, w.low), (21.3, 11.0));
    }

    /// Hits the real API: `cargo test -p familiar-sys -- --ignored live`.
    #[test]
    #[ignore]
    fn live_fetch() {
        let w = fetch("Bogotá").unwrap();
        assert!(!w.hours.is_empty());
        println!("{w:?}");
    }

    #[test]
    fn encodes_city_names() {
        assert_eq!(encode("Bogotá D.C"), "Bogot%C3%A1%20D.C");
    }
}
