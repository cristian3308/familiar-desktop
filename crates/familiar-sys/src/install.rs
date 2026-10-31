//! Per-user install: the executable goes to
//! `%LOCALAPPDATA%\Programs\FamiliarCore` and a shortcut to the Start
//! menu. No admin rights, no registry beyond the optional Run entry.

use std::path::{Path, PathBuf};

use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
use windows::core::{HSTRING, Interface};

use crate::com::ComGuard;

pub const EXE_NAME: &str = "familiar-core.exe";
const SHORTCUT: &str = "Familiar Core.lnk";

/// `%LOCALAPPDATA%\Programs\FamiliarCore`.
pub fn install_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Programs").join("FamiliarCore"))
}

/// `...\Start Menu\Programs\Familiar Core.lnk`.
pub fn shortcut_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join(r"Microsoft\Windows\Start Menu\Programs").join(SHORTCUT))
}

/// Writes a `.lnk` pointing at `target`.
pub fn create_shortcut(lnk: &Path, target: &Path, description: &str) -> windows::core::Result<()> {
    let _com = ComGuard::sta();
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.SetPath(&HSTRING::from(target.as_os_str()))?;
        if let Some(dir) = target.parent() {
            link.SetWorkingDirectory(&HSTRING::from(dir.as_os_str()))?;
        }
        link.SetDescription(&HSTRING::from(description))?;
        let file: IPersistFile = link.cast()?;
        file.Save(&HSTRING::from(lnk.as_os_str()), true)
    }
}

/// Copies the running executable into the install folder and adds the
/// Start menu shortcut. Returns the installed executable.
pub fn install() -> std::io::Result<PathBuf> {
    let dir = install_dir().ok_or_else(|| std::io::Error::other("LOCALAPPDATA is not set"))?;
    std::fs::create_dir_all(&dir)?;
    let target = dir.join(EXE_NAME);
    let me = std::env::current_exe()?;
    if !same_file(&me, &target) {
        std::fs::copy(&me, &target)?;
    }
    if let Some(lnk) = shortcut_path() {
        create_shortcut(&lnk, &target, "Familiar Core").map_err(std::io::Error::other)?;
    }
    Ok(target)
}

/// Removes the shortcut and the installed executable (unless it is the one
/// running). Settings and themes in %APPDATA% are kept.
pub fn uninstall() -> std::io::Result<()> {
    if let Some(lnk) = shortcut_path()
        && lnk.exists()
    {
        std::fs::remove_file(lnk)?;
    }
    if let Some(dir) = install_dir() {
        let exe = dir.join(EXE_NAME);
        let me = std::env::current_exe()?;
        if exe.exists() && !same_file(&me, &exe) {
            std::fs::remove_file(&exe)?;
            let _ = std::fs::remove_dir(&dir);
        }
    }
    Ok(())
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_per_user() {
        if let Some(dir) = install_dir() {
            assert!(dir.ends_with(r"Programs\FamiliarCore"));
        }
        if let Some(lnk) = shortcut_path() {
            assert!(lnk.to_string_lossy().ends_with(r"Start Menu\Programs\Familiar Core.lnk"));
        }
    }

    #[test]
    fn shortcut_round_trip() {
        let dir = std::env::temp_dir().join(format!("familiar-lnk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let lnk = dir.join("test.lnk");
        let target = std::env::current_exe().unwrap();
        create_shortcut(&lnk, &target, "test").unwrap();
        assert!(lnk.metadata().unwrap().len() > 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
