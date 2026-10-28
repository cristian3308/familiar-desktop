//! Installed applications via the `shell:AppsFolder` virtual folder, which
//! lists Win32 shortcuts and packaged (UWP/MSIX) apps in one place — the
//! same source the Start menu uses.

use windows::Win32::Foundation::HWND;
use windows::Win32::Storage::EnhancedStorage::PKEY_Link_TargetParsingPath;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IShellItem, IShellItem2, KF_FLAG_DEFAULT,
    SHGetKnownFolderItem, SHGetKnownFolderPath, SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{GUID, HSTRING, Interface, PCWSTR, w};

use crate::com::ComGuard;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledApp {
    pub name: String,
    /// Parsing name inside AppsFolder: an AUMID for packaged apps, or a
    /// path (possibly prefixed by a known-folder GUID) for Win32 apps.
    pub parsing_name: String,
    /// Resolved `.exe` path when known (used to match running windows).
    pub exe_path: String,
    pub packaged: bool,
}

fn display_name(item: &IShellItem, kind: windows::Win32::UI::Shell::SIGDN) -> Option<String> {
    unsafe {
        let p = item.GetDisplayName(kind).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s
    }
}

/// Turns `{6D809377-...}\Microsoft VS Code\Code.exe` into a real path.
pub fn resolve_known_folder(path: &str) -> String {
    let Some(rest) = path.strip_prefix('{') else { return path.to_string() };
    let Some((guid, tail)) = rest.split_once('}') else { return path.to_string() };
    let Ok(guid) = GUID::try_from(guid) else { return path.to_string() };
    unsafe {
        match SHGetKnownFolderPath(&guid, KF_FLAG_DEFAULT, None) {
            Ok(p) => {
                let base = p.to_string().unwrap_or_default();
                CoTaskMemFree(Some(p.0 as *const _));
                format!("{base}{tail}")
            }
            Err(_) => path.to_string(),
        }
    }
}

fn looks_like_noise(name: &str, parsing: &str) -> bool {
    let n = name.to_lowercase();
    let p = parsing.to_lowercase();
    n.contains("uninstall")
        || n.starts_with("readme")
        || p.starts_with("http://")
        || p.starts_with("https://")
        || p.ends_with(".url")
        || p.ends_with(".txt")
        || p.ends_with(".chm")
        || p.ends_with(".pdf")
}

/// Enumerates AppsFolder. Call from a background thread: it can take a few
/// hundred milliseconds the first time.
pub fn installed_apps() -> windows::core::Result<Vec<InstalledApp>> {
    let _com = ComGuard::sta();
    let mut out = Vec::new();
    unsafe {
        let folder: IShellItem = SHGetKnownFolderItem(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None)?;
        let items: IEnumShellItems = folder.BindToHandler(None, &BHID_EnumItems)?;
        loop {
            let mut batch = [None];
            let mut fetched = 0u32;
            if items.Next(&mut batch, Some(&mut fetched)).is_err() || fetched == 0 {
                break;
            }
            let Some(item) = batch[0].take() else { break };
            let (Some(name), Some(parsing)) =
                (display_name(&item, SIGDN_NORMALDISPLAY), display_name(&item, SIGDN_PARENTRELATIVEPARSING))
            else {
                continue;
            };
            if name.is_empty() || looks_like_noise(&name, &parsing) {
                continue;
            }
            let packaged = parsing.contains('!') && !parsing.contains('\\');
            let mut exe_path = String::new();
            if let Ok(item2) = item.cast::<IShellItem2>()
                && let Ok(p) = item2.GetString(&PKEY_Link_TargetParsingPath)
            {
                exe_path = p.to_string().unwrap_or_default();
                CoTaskMemFree(Some(p.0 as *const _));
            }
            if exe_path.is_empty() && !packaged {
                exe_path = resolve_known_folder(&parsing);
            }
            if !exe_path.to_lowercase().ends_with(".exe") {
                exe_path.clear();
            }
            out.push(InstalledApp { name, parsing_name: parsing, exe_path, packaged });
        }
    }
    out.sort_by_key(|a| a.name.to_lowercase());
    out.dedup_by(|a, b| a.parsing_name == b.parsing_name);
    Ok(out)
}

/// Launches an AppsFolder entry (works for both Win32 and packaged apps), a
/// plain path, or a URI such as `ms-settings:batterysaver`.
pub fn launch(target: &str) -> windows::core::Result<()> {
    let file =
        if is_path(target) || is_uri(target) { target.to_string() } else { format!("shell:AppsFolder\\{target}") };
    let file = HSTRING::from(file);
    let r =
        unsafe { ShellExecuteW(Some(HWND::default()), w!("open"), PCWSTR(file.as_ptr()), None, None, SW_SHOWNORMAL) };
    // ShellExecute returns a pseudo-HINSTANCE; > 32 means success.
    if r.0 as usize > 32 { Ok(()) } else { Err(windows::core::Error::from_thread()) }
}

fn is_path(target: &str) -> bool {
    target.contains(":\\") || target.starts_with("\\\\")
}

fn is_uri(target: &str) -> bool {
    target.split_once(':').is_some_and(|(scheme, _)| {
        scheme.len() > 1 && scheme.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
    })
}
