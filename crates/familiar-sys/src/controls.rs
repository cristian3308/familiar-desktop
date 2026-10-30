//! Quick settings for the Control Center: volume, Wi-Fi/Bluetooth radios,
//! the current network and display brightness.
//!
//! Volume calls are cheap COM calls; radios, network and brightness can
//! block for a while and must be called from a worker thread.

use windows::Devices::Radios::{Radio, RadioKind, RadioState};
use windows::Networking::Connectivity::NetworkInformation;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{IMMDeviceEnumerator, MMDeviceEnumerator, eConsole, eRender};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};

use crate::com::ComGuard;

fn endpoint_volume() -> Option<IAudioEndpointVolume> {
    unsafe {
        let devices: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let device = devices.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
        device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok()
    }
}

/// Master volume (0.0–1.0) and mute state of the default output.
pub fn volume() -> Option<(f32, bool)> {
    let _com = ComGuard::sta();
    let v = endpoint_volume()?;
    unsafe { Some((v.GetMasterVolumeLevelScalar().ok()?, v.GetMute().ok()?.as_bool())) }
}

pub fn set_volume(level: f32) {
    let _com = ComGuard::sta();
    if let Some(v) = endpoint_volume() {
        unsafe {
            let _ = v.SetMasterVolumeLevelScalar(level.clamp(0.0, 1.0), std::ptr::null());
            if level > 0.0 {
                let _ = v.SetMute(false, std::ptr::null());
            }
        }
    }
}

pub fn set_mute(muted: bool) {
    let _com = ComGuard::sta();
    if let Some(v) = endpoint_volume() {
        unsafe {
            let _ = v.SetMute(muted, std::ptr::null());
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Radios {
    /// `None` when the machine has no such radio.
    pub wifi: Option<bool>,
    pub bluetooth: Option<bool>,
    /// Name of the connection used for the internet (SSID or "Ethernet").
    pub network: String,
}

fn radios_of(kind: RadioKind) -> Vec<Radio> {
    let Ok(all) = Radio::GetRadiosAsync().and_then(|op| op.join()) else { return Vec::new() };
    all.into_iter().filter(|r| r.Kind().is_ok_and(|k| k == kind)).collect()
}

/// Blocking: call from a worker thread.
pub fn radios() -> Radios {
    let _com = ComGuard::mta();
    let state = |kind| {
        let list = radios_of(kind);
        (!list.is_empty()).then(|| list.iter().any(|r| r.State().is_ok_and(|s| s == RadioState::On)))
    };
    let network = NetworkInformation::GetInternetConnectionProfile()
        .and_then(|p| p.ProfileName())
        .map(|n| n.to_string())
        .unwrap_or_default();
    Radios { wifi: state(RadioKind::WiFi), bluetooth: state(RadioKind::Bluetooth), network }
}

/// Turns every Wi-Fi (or Bluetooth) radio on or off. Blocking.
pub fn set_radio(wifi: bool, on: bool) {
    let _com = ComGuard::mta();
    if Radio::RequestAccessAsync().and_then(|op| op.join()).is_err() {
        log::warn!("radio access denied");
        return;
    }
    let kind = if wifi { RadioKind::WiFi } else { RadioKind::Bluetooth };
    let state = if on { RadioState::On } else { RadioState::Off };
    for r in radios_of(kind) {
        if let Err(e) = r.SetStateAsync(state).and_then(|op| op.join()) {
            log::warn!("setting radio: {e}");
        }
    }
}

fn powershell(script: &str) -> Option<String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Built-in display brightness in percent (laptops); `None` on desktops.
/// Blocking (WMI through PowerShell, ~0.5 s).
pub fn brightness() -> Option<u32> {
    powershell(
        "(Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightness -ErrorAction Stop).CurrentBrightness",
    )?
    .lines()
    .next()?
    .trim()
    .parse()
    .ok()
}

/// Blocking.
pub fn set_brightness(percent: u32) {
    let script = format!(
        "Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightnessMethods | \
         Invoke-CimMethod -MethodName WmiSetBrightness -Arguments @{{Timeout=1;Brightness={}}} | Out-Null",
        percent.min(100)
    );
    if powershell(&script).is_none() {
        log::warn!("setting brightness failed");
    }
}

#[cfg(test)]
mod tests {
    /// Reads the live state: `cargo test -p familiar-sys -- --ignored controls`.
    #[test]
    #[ignore]
    fn controls_live_read() {
        println!("volume {:?}", super::volume());
        println!("radios {:?}", super::radios());
        println!("brightness {:?}", super::brightness());
    }
}
