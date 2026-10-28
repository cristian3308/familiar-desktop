//! High-resolution icon extraction.
//!
//! `IShellItemImageFactory::GetImage` returns the best icon the shell has
//! (up to 256x256, including packaged-app logos), unlike `ExtractIconEx`
//! which tops out at 32/48 px for most apps.

use std::path::Path;

use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject,
    GetDIBits, GetObjectW, HBITMAP, HGDIOBJ,
};
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY,
};
use windows::core::{HSTRING, PCWSTR};

use crate::com::ComGuard;

pub enum IconSource<'a> {
    /// An AppsFolder parsing name (AUMID or path).
    AppsFolder(&'a str),
    /// Any file system path (`.exe`, `.lnk`, folder...).
    Path(&'a str),
}

/// Straight-alpha RGBA pixels.
pub struct IconImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub fn extract(source: IconSource<'_>, size: u32) -> windows::core::Result<IconImage> {
    let _com = ComGuard::sta();
    let parsing = match source {
        IconSource::AppsFolder(p) => format!("shell:AppsFolder\\{p}"),
        IconSource::Path(p) => p.to_string(),
    };
    let parsing = HSTRING::from(parsing);
    unsafe {
        let factory: IShellItemImageFactory = SHCreateItemFromParsingName(PCWSTR(parsing.as_ptr()), None)?;
        let bmp = factory.GetImage(SIZE { cx: size as i32, cy: size as i32 }, SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK)?;
        let img = bitmap_to_rgba(bmp);
        let _ = DeleteObject(HGDIOBJ(bmp.0));
        img
    }
}

/// Extracts and writes a PNG; returns `true` on success.
pub fn extract_png(source: IconSource<'_>, size: u32, out: &Path) -> bool {
    let Ok(img) = extract(source, size) else { return false };
    if let Some(dir) = out.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    image::save_buffer(out, &img.rgba, img.width, img.height, image::ColorType::Rgba8).is_ok()
}

unsafe fn bitmap_to_rgba(bmp: HBITMAP) -> windows::core::Result<IconImage> {
    let mut info = BITMAP::default();
    if unsafe { GetObjectW(HGDIOBJ(bmp.0), std::mem::size_of::<BITMAP>() as i32, Some(&mut info as *mut _ as *mut _)) }
        == 0
    {
        return Err(windows::core::Error::from_thread());
    }
    let (w, h) = (info.bmWidth as u32, info.bmHeight.unsigned_abs());
    let mut bi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            biHeight: -(h as i32), // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut px = vec![0u8; (w * h * 4) as usize];
    let dc = unsafe { CreateCompatibleDC(None) };
    let lines = unsafe { GetDIBits(dc, bmp, 0, h, Some(px.as_mut_ptr() as *mut _), &mut bi, DIB_RGB_COLORS) };
    unsafe {
        let _ = DeleteDC(dc);
    }
    if lines == 0 {
        return Err(windows::core::Error::from_thread());
    }
    // BGRA -> RGBA. The shell hands out premultiplied alpha; undo it so the
    // PNG (and the UI toolkit) see straight alpha.
    let premultiplied = px.chunks_exact(4).all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]);
    let has_alpha = px.chunks_exact(4).any(|p| p[3] != 0);
    for p in px.chunks_exact_mut(4) {
        p.swap(0, 2);
        if !has_alpha {
            p[3] = 255;
        } else if premultiplied && p[3] != 0 && p[3] != 255 {
            let a = p[3] as u32;
            for c in &mut p[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    Ok(IconImage { width: w, height: h, rgba: px })
}
