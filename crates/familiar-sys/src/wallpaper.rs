//! The desktop wallpaper: reading the current image (for themes built from
//! it), and switching between the images of a folder (Alt+W).
//!
//! Wallpaper engines such as Lively draw their own window over the desktop,
//! so a change made here is only visible once they are paused or closed.

use std::path::{Path, PathBuf};

use windows::Win32::UI::WindowsAndMessaging::{
    SPI_GETDESKWALLPAPER, SPI_SETDESKWALLPAPER, SPIF_SENDCHANGE, SPIF_UPDATEINIFILE,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
};
use windows::core::HSTRING;

const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "bmp"];

/// Path of the current wallpaper image, if it is a file.
pub fn current() -> Option<PathBuf> {
    let mut buf = [0u16; 1024];
    unsafe {
        SystemParametersInfoW(
            SPI_GETDESKWALLPAPER,
            buf.len() as u32,
            Some(buf.as_mut_ptr().cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .ok()?;
    }
    let len = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    let path = PathBuf::from(String::from_utf16_lossy(&buf[..len]));
    path.is_file().then_some(path)
}

/// Sets the wallpaper for the user (persisted, like the Settings app does).
pub fn set(path: &Path) -> windows::core::Result<()> {
    let wide = HSTRING::from(path.as_os_str());
    unsafe {
        SystemParametersInfoW(
            SPI_SETDESKWALLPAPER,
            0,
            Some(wide.as_ptr() as *mut _),
            SPIF_UPDATEINIFILE | SPIF_SENDCHANGE,
        )
    }
}

/// Straight-alpha RGBA image.
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Decodes `path` and shrinks it to fit within `max` x `max` pixels.
pub fn load(path: &Path, max: u32) -> Option<Picture> {
    let img = image::open(path).ok()?;
    let img = if img.width() > max || img.height() > max { img.thumbnail(max, max) } else { img };
    let rgba = img.to_rgba8();
    Some(Picture { width: rgba.width(), height: rgba.height(), rgba: rgba.into_raw() })
}

/// Decodes `path` cropped and scaled to exactly `w` x `h` ("fill"), as the
/// desktop shows it.
pub fn load_fill(path: &Path, w: u32, h: u32) -> Option<Picture> {
    let img = image::open(path).ok()?;
    let filled = img.resize_to_fill(w, h, image::imageops::FilterType::Triangle).to_rgba8();
    Some(Picture { width: filled.width(), height: filled.height(), rgba: filled.into_raw() })
}

/// Images in `dir`, sorted by name.
pub fn images_in(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| {
                    p.extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| IMAGE_EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort_by_key(|p| p.to_string_lossy().to_lowercase());
    files
}

/// The image after `current` in `list` (the first one when `current` is
/// not in it), wrapping around.
pub fn next_after<'a>(list: &'a [PathBuf], current: Option<&Path>) -> Option<&'a PathBuf> {
    let i = current.and_then(|c| list.iter().position(|p| p.as_path().eq(c)));
    match i {
        Some(i) => list.get((i + 1) % list.len()),
        None => list.first(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_wraps() {
        let list: Vec<PathBuf> = ["a.jpg", "b.jpg", "c.jpg"].iter().map(PathBuf::from).collect();
        assert_eq!(next_after(&list, Some(Path::new("a.jpg"))).unwrap(), Path::new("b.jpg"));
        assert_eq!(next_after(&list, Some(Path::new("c.jpg"))).unwrap(), Path::new("a.jpg"));
        assert_eq!(next_after(&list, Some(Path::new("zzz.jpg"))).unwrap(), Path::new("a.jpg"));
        assert_eq!(next_after(&list, None).unwrap(), Path::new("a.jpg"));
        assert!(next_after(&[], None).is_none());
    }

    #[test]
    #[ignore = "reads the real wallpaper"]
    fn reads_current_wallpaper() {
        let p = current();
        println!("{p:?}");
        if let Some(p) = p {
            let pic = load(&p, 64).unwrap();
            assert!(pic.width <= 64 && pic.height <= 64);
        }
    }
}
