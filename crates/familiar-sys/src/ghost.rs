//! Window "ghosts" for the open/minimise animations.
//!
//! A ghost is a live DWM thumbnail of another app's window, shown in a
//! transparent, click-through host window. Moving and scaling it is just a
//! `DwmUpdateThumbnailProperties` call: DWM composites it on the GPU, so the
//! animation costs no CPU rendering at all (the previous software-rendered
//! overlay repainted a full-screen bitmap every frame and stuttered).
//!
//! While the ghost flies, the real window can be made fully transparent
//! (layered, alpha 0); DWM still renders its thumbnail, so the user sees a
//! single window zooming instead of a window plus a copy. The original
//! extended style is always put back.
//!
//! For the genie effect the ghost is made of several thumbnails of the same
//! window, each showing one horizontal strip (`rcSource`) at its own
//! destination, which lets the window bend into the dock icon.

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWM_THUMBNAIL_PROPERTIES, DWM_TNP_OPACITY, DWM_TNP_RECTDESTINATION, DWM_TNP_RECTSOURCE,
    DWM_TNP_SOURCECLIENTAREAONLY, DWM_TNP_VISIBLE, DwmFlush, DwmQueryThumbnailSourceSize, DwmRegisterThumbnail,
    DwmUnregisterThumbnail, DwmUpdateThumbnailProperties,
};
use windows::Win32::Graphics::Gdi::{BLACK_BRUSH, GetStockObject, HBRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GWL_EXSTYLE, GetWindowLongPtrW, HWND_TOPMOST, IsWindow, LWA_ALPHA, LWA_COLORKEY,
    RegisterClassW, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SetLayeredWindowAttributes, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::w;

use crate::Hwnd;
use crate::window::Rect;

unsafe extern "system" fn host_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    crate::safety::ffi_guard(LRESULT(0), || unsafe { DefWindowProcW(hwnd, msg, wp, lp) })
}

pub struct Ghost {
    host: HWND,
    frames: Frames,
    /// Window made transparent for the flight, with its original ex-style.
    hidden: Option<(HWND, isize)>,
}

/// The thumbnails of a flight, detached from the host window so frames can
/// be placed from an animation thread (DWM calls are thread-agnostic).
/// Handles stay valid until [`Ghost::end`]; updating one after that just
/// fails quietly.
#[derive(Clone, Debug, Default)]
pub struct Frames {
    /// Top-left of the host in screen pixels; thumbnail rects are relative.
    origin: (i32, i32),
    thumbs: Vec<isize>,
    /// The part of the source that is the visible window: DWM's source
    /// includes the invisible resize borders, which would otherwise shrink
    /// the content and make it jump when the real window takes over.
    visible: RECT,
}

impl Frames {
    pub fn strips(&self) -> usize {
        self.thumbs.len()
    }

    /// Places the whole visible window at `dest` (screen pixels).
    pub fn frame(&self, dest: Rect, opacity: u8) {
        self.place(0, self.visible, dest, Some(opacity));
    }

    /// Places strip `i` of `strips()` (top to bottom) at `dest`. With
    /// `opacity` `None` only the destination is sent to DWM, which is much
    /// cheaper when hundreds of strips move every frame.
    pub fn frame_strip(&self, i: usize, dest: Rect, opacity: Option<u8>) {
        let n = self.thumbs.len().max(1) as i32;
        let v = self.visible;
        let h = v.bottom - v.top;
        let i = i as i32;
        let src = RECT { left: v.left, top: v.top + h * i / n, right: v.right, bottom: v.top + h * (i + 1) / n };
        self.place(i as usize, src, dest, opacity);
    }

    fn place(&self, i: usize, src: RECT, dest: Rect, opacity: Option<u8>) {
        let Some(&thumb) = self.thumbs.get(i) else { return };
        let (ox, oy) = self.origin;
        let flags = match opacity {
            Some(_) => {
                DWM_TNP_RECTDESTINATION
                    | DWM_TNP_RECTSOURCE
                    | DWM_TNP_OPACITY
                    | DWM_TNP_VISIBLE
                    | DWM_TNP_SOURCECLIENTAREAONLY
            }
            None => DWM_TNP_RECTDESTINATION,
        };
        let props = DWM_THUMBNAIL_PROPERTIES {
            dwFlags: flags,
            rcDestination: RECT {
                left: dest.x - ox,
                top: dest.y - oy,
                right: dest.x - ox + dest.w.max(1),
                bottom: dest.y - oy + dest.h.max(1),
            },
            rcSource: src,
            opacity: opacity.unwrap_or(255),
            fVisible: true.into(),
            fSourceClientAreaOnly: false.into(),
        };
        unsafe {
            let _ = DwmUpdateThumbnailProperties(thumb, &props);
        }
    }
}

/// Visible part of a `w` x `h` thumbnail source whose on-screen window is
/// `visible_w` x `visible_h`: Windows 11 adds invisible borders left, right
/// and bottom (and on top too when maximised).
fn visible_part(w: i32, h: i32, visible_w: i32, visible_h: i32) -> RECT {
    let side = ((w - visible_w) / 2).max(0);
    let spare_h = (h - visible_h).max(0);
    // Maximised windows overhang the monitor by the same amount everywhere.
    let top = if side >= 8 && spare_h >= 2 * side { side } else { 0 };
    let bottom = (spare_h - top).max(0);
    RECT { left: side, top, right: (w - side).max(side + 1), bottom: (h - bottom).max(top + 1) }
}

impl Ghost {
    /// Creates the (hidden) host window. Call on the UI thread.
    pub fn new() -> Option<Self> {
        unsafe {
            let hinstance = GetModuleHandleW(None).ok()?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(host_proc),
                hInstance: hinstance.into(),
                lpszClassName: w!("FamiliarCoreGhost"),
                hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
                ..Default::default()
            };
            RegisterClassW(&class);
            let host = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
                w!("FamiliarCoreGhost"),
                w!("Familiar Ghost"),
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
            // Black is keyed out: only the thumbnail is visible.
            SetLayeredWindowAttributes(host, COLORREF(0), 0, LWA_COLORKEY).ok()?;
            Some(Self { host, frames: Frames::default(), hidden: None })
        }
    }

    /// Starts showing `source` inside `area` (screen pixels). Returns false if
    /// DWM refused the thumbnail (e.g. the window is gone). `visible` is the
    /// window's on-screen size, used to crop the invisible borders.
    /// `strips` thumbnails are registered (1 for a plain zoom).
    pub fn begin(&mut self, source: Hwnd, area: Rect, visible: (i32, i32), hide_source: bool, strips: usize) -> bool {
        self.end();
        let mut thumbs = Vec::new();
        unsafe {
            let _ = SetWindowPos(self.host, Some(HWND_TOPMOST), area.x, area.y, area.w, area.h, SWP_NOACTIVATE);
            for _ in 0..strips.max(1) {
                match DwmRegisterThumbnail(self.host, source.get()) {
                    Ok(t) => thumbs.push(t),
                    Err(_) => break,
                }
            }
            let Some(&first) = thumbs.first() else { return false };
            let (w, h) = DwmQueryThumbnailSourceSize(first).map(|s| (s.cx.max(1), s.cy.max(1))).unwrap_or((1, 1));
            self.frames =
                Frames { origin: (area.x, area.y), thumbs, visible: visible_part(w, h, visible.0, visible.1) };
            let _ = ShowWindow(self.host, SW_SHOWNOACTIVATE);
            // Windows that are already layered manage their own alpha;
            // leave those alone and just fly the ghost over them.
            let ex = GetWindowLongPtrW(source.get(), GWL_EXSTYLE);
            if hide_source && ex & WS_EX_LAYERED.0 as isize == 0 {
                SetWindowLongPtrW(source.get(), GWL_EXSTYLE, ex | WS_EX_LAYERED.0 as isize);
                let _ = SetLayeredWindowAttributes(source.get(), COLORREF(0), 0, LWA_ALPHA);
                self.hidden = Some((source.get(), ex));
            }
        }
        true
    }

    /// Waits for DWM's next composition so frames land on v-sync instead
    /// of whenever a timer happens to fire.
    pub fn vsync() {
        unsafe {
            let _ = DwmFlush();
        }
    }

    /// The thumbnails, for drawing frames (from any thread).
    pub fn frames(&self) -> Frames {
        self.frames.clone()
    }

    /// Removes the thumbnails (and hides the host) but keeps the source
    /// window transparent until [`end`](Self::end).
    pub fn drop_thumbnails(&mut self) {
        if !self.frames.thumbs.is_empty() {
            unsafe {
                for t in self.frames.thumbs.drain(..) {
                    let _ = DwmUnregisterThumbnail(t);
                }
                let _ = ShowWindow(self.host, SW_HIDE);
            }
        }
    }

    pub fn end(&mut self) {
        if let Some((h, ex)) = self.hidden.take() {
            unsafe {
                if IsWindow(Some(h)).as_bool() {
                    SetWindowLongPtrW(h, GWL_EXSTYLE, ex);
                }
            }
        }
        if !self.frames.thumbs.is_empty() {
            unsafe {
                for t in self.frames.thumbs.drain(..) {
                    let _ = DwmUnregisterThumbnail(t);
                }
                let _ = ShowWindow(self.host, SW_HIDE);
            }
        }
    }
}

impl Drop for Ghost {
    fn drop(&mut self) {
        self.end();
    }
}

#[cfg(test)]
mod tests {
    use super::visible_part;

    #[test]
    fn crops_invisible_borders() {
        // Normal window: 7 px left, right and bottom.
        let r = visible_part(1114, 707, 1100, 700);
        assert_eq!((r.left, r.top, r.right, r.bottom), (7, 0, 1107, 700));
        // Maximised: 8 px everywhere.
        let r = visible_part(1936, 986, 1920, 970);
        assert_eq!((r.left, r.top, r.right, r.bottom), (8, 8, 1928, 978));
        // Already the visible size: nothing cropped.
        let r = visible_part(800, 600, 800, 600);
        assert_eq!((r.left, r.top, r.right, r.bottom), (0, 0, 800, 600));
    }
}
