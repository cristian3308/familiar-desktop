//! A click-through, topmost layered window drawn straight from a CPU pixel
//! buffer with one `UpdateLayeredWindow` call per frame. Used for the genie
//! effect, where hundreds of DWM thumbnail strips were far too slow.

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, BitBlt, CAPTUREBLT,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, HBITMAP, HDC, HGDIOBJ,
    ReleaseDC, SRCCOPY, SelectObject,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, HWND_TOPMOST, RegisterClassW, SW_HIDE, SW_SHOWNOACTIVATE,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos, ShowWindow, ULW_ALPHA, UpdateLayeredWindow, WNDCLASSW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::w;

use crate::window::Rect;

unsafe extern "system" fn canvas_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    crate::safety::ffi_guard(LRESULT(0), || unsafe { DefWindowProcW(hwnd, msg, wp, lp) })
}

/// A 32-bit top-down DIB selected into a memory DC.
struct Dib {
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u8,
    w: i32,
    h: i32,
}

impl Dib {
    fn new(w: i32, h: i32) -> Option<Self> {
        unsafe {
            let screen = GetDC(None);
            let dc = CreateCompatibleDC(Some(screen));
            ReleaseDC(None, screen);
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let bitmap = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
            let old = SelectObject(dc, HGDIOBJ(bitmap.0));
            Some(Self { dc, bitmap, old, bits: bits as *mut u8, w, h })
        }
    }

    fn pixels(&mut self) -> &mut [u8] {
        // SAFETY: the DIB section owns w*h*4 bytes for as long as it lives.
        unsafe { std::slice::from_raw_parts_mut(self.bits, (self.w * self.h * 4) as usize) }
    }
}

impl Drop for Dib {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
            let _ = DeleteDC(self.dc);
        }
    }
}

/// Copies `r` of the screen as composed by DWM (layered windows included).
/// Returns BGRA pixels with alpha forced to 255.
pub fn capture_screen(r: Rect) -> Option<Vec<u8>> {
    let mut dib = Dib::new(r.w.max(1), r.h.max(1))?;
    unsafe {
        let screen = GetDC(None);
        let ok = BitBlt(dib.dc, 0, 0, r.w, r.h, Some(screen), r.x, r.y, SRCCOPY | CAPTUREBLT).is_ok();
        ReleaseDC(None, screen);
        if !ok {
            return None;
        }
    }
    let mut px = dib.pixels().to_vec();
    for p in px.as_chunks_mut::<4>().0 {
        p[3] = 255;
    }
    Some(px)
}

/// The window plus a back buffer sized to it.
pub struct Canvas {
    hwnd: HWND,
    dib: Option<Dib>,
    rect: Rect,
}

// SAFETY: the window and DIB are only touched by one thread at a time: the
// UI thread creates and hides it, the animation thread draws in between.
unsafe impl Send for Canvas {}

impl Canvas {
    pub fn new() -> Option<Self> {
        unsafe {
            let hinstance = GetModuleHandleW(None).ok()?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(canvas_proc),
                hInstance: hinstance.into(),
                lpszClassName: w!("FamiliarCoreCanvas"),
                ..Default::default()
            };
            RegisterClassW(&class);
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
                w!("FamiliarCoreCanvas"),
                w!("Familiar Canvas"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                Some(hinstance.into()),
                None,
            )
            .ok()?;
            Some(Self { hwnd, dib: None, rect: Rect::default() })
        }
    }

    /// Prepares a buffer covering `rect` (screen pixels).
    pub fn begin(&mut self, rect: Rect) -> bool {
        if self.dib.as_ref().is_none_or(|d| d.w != rect.w || d.h != rect.h) {
            self.dib = Dib::new(rect.w.max(1), rect.h.max(1));
        }
        self.rect = rect;
        self.dib.is_some()
    }

    /// Size of the buffer; pixels are BGRA, premultiplied, top-down.
    pub fn size(&self) -> (usize, usize) {
        (self.rect.w.max(0) as usize, self.rect.h.max(0) as usize)
    }

    pub fn origin(&self) -> (i32, i32) {
        (self.rect.x, self.rect.y)
    }

    /// Fills the buffer with `draw` and puts it on screen.
    pub fn present(&mut self, draw: impl FnOnce(&mut [u8])) {
        let Some(dib) = self.dib.as_mut() else { return };
        draw(dib.pixels());
        let pos = POINT { x: self.rect.x, y: self.rect.y };
        let size = SIZE { cx: dib.w, cy: dib.h };
        let src = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        unsafe {
            let screen = GetDC(None);
            let _ = UpdateLayeredWindow(
                self.hwnd,
                Some(screen),
                Some(&pos),
                Some(&size),
                Some(dib.dc),
                Some(&src),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            );
            ReleaseDC(None, screen);
        }
    }

    /// Shows the window on top. Call from the UI thread (it owns the
    /// window; cross-thread z-order changes would wait on it).
    pub fn show(&self) {
        unsafe {
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
        }
    }

    pub fn hide(&mut self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        // Large buffers are not kept between animations.
        self.dib = None;
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        self.dib = None;
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
