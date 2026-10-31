//! Recently opened documents, from the user's Recent folder
//! (`FOLDERID_Recent`, the same list File Explorer's "Recent" shows).
//!
//! Each entry is a shortcut; opening the `.lnk` with the shell opens its
//! target, so the targets never need to be resolved.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{FOLDERID_Recent, KF_FLAG_DEFAULT, SHGetKnownFolderPath};

#[derive(Clone, Debug, PartialEq)]
pub struct RecentFile {
    /// Display name: the target's file name ("informe.docx").
    pub name: String,
    /// The shortcut in the Recent folder; launch this.
    pub link: PathBuf,
    pub modified: SystemTime,
    /// Shortcuts to folders have no extension in their name.
    pub is_folder: bool,
}

fn recent_dir() -> Option<PathBuf> {
    unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_Recent, KF_FLAG_DEFAULT, None).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s.map(PathBuf::from)
    }
}

/// The `limit` most recent entries, newest first.
pub fn recent_files(limit: usize) -> Vec<RecentFile> {
    let Some(dir) = recent_dir() else { return Vec::new() };
    let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut files: Vec<RecentFile> = entries.filter_map(|e| e.ok()).filter_map(|e| entry(&e.path())).collect();
    sort_newest_first(&mut files);
    files.truncate(limit);
    files
}

fn entry(path: &Path) -> Option<RecentFile> {
    let ext = path.extension()?.to_str()?;
    if !ext.eq_ignore_ascii_case("lnk") {
        return None;
    }
    let name = path.file_stem()?.to_string_lossy().into_owned();
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
    Some(RecentFile { is_folder: !looks_like_file(&name), name, link: path.to_path_buf(), modified })
}

/// "report.pdf" is a file, "Descargas" a folder.
fn looks_like_file(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty() && (1..=5).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_alphanumeric())
    })
}

fn sort_newest_first(files: &mut [RecentFile]) {
    files.sort_by_key(|f| std::cmp::Reverse(f.modified));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn files_and_folders() {
        assert!(looks_like_file("informe.docx"));
        assert!(looks_like_file("foto.JPG"));
        assert!(!looks_like_file("Descargas"));
        assert!(!looks_like_file(".gitignore"));
        assert!(!looks_like_file("v1.2 notas largas"));
    }

    #[test]
    fn newest_first() {
        let at = |s| SystemTime::UNIX_EPOCH + Duration::from_secs(s);
        let f = |n: &str, s| RecentFile { name: n.into(), link: n.into(), modified: at(s), is_folder: false };
        let mut v = vec![f("a", 1), f("c", 3), f("b", 2)];
        sort_newest_first(&mut v);
        let names: Vec<_> = v.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["c", "b", "a"]);
    }

    #[test]
    #[ignore = "reads the real Recent folder"]
    fn lists_recent_files() {
        let files = recent_files(10);
        println!("{files:#?}");
    }
}
