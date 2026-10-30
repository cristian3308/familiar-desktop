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
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

pub struct Ghost {
    host: HWND,
    /// Top-left of the host in screen pixels; thumbnail rects are relative.
    origin: (i32, i32),
    thumbs: Vec<isize>,
    /// Size of the source as DWM sees it (for strip source rects).
    source_size: (i32, i32),
    /// Window made transparent for the flight, with its original ex-style.
    hidden: Option<(HWND, isize)>,
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
            Some(Self { host, origin: (0, 0), thumbs: Vec::new(), source_size: (1, 1), hidden: None })
        }
    }

    /// Starts showing `source` inside `area` (screen pixels). Returns false if
    /// DWM refused the thumbnail (e.g. the window is gone).
    /// `strips` thumbnails are registered (1 for a plain zoom).
    pub fn begin(&mut self, source: Hwnd, area: Rect, hide_source: bool, strips: usize) -> bool {
        self.end();
        unsafe {
            let _ = SetWindowPos(self.host, Some(HWND_TOPMOST), area.x, area.y, area.w, area.h, SWP_NOACTIVATE);
            for _ in 0..strips.max(1) {
                match DwmRegisterThumbnail(self.host, source.get()) {
                    Ok(t) => self.thumbs.push(t),
                    Err(_) => break,
                }
            }
            let Some(&first) = self.thumbs.first() else { return false };
            self.source_size = DwmQueryThumbnailSourceSize(first).map(|s| (s.cx.max(1), s.cy.max(1))).unwrap_or((1, 1));
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
        self.origin = (area.x, area.y);
        true
    }

    /// Waits for DWM's next composition so frames land on v-sync instead
    /// of whenever the timer happens to fire.
    pub fn vsync() {
        unsafe {
            let _ = DwmFlush();
        }
    }

    pub fn strips(&self) -> usize {
        self.thumbs.len()
    }

    /// Places the whole thumbnail at `dest` (screen pixels) with `opacity`.
    pub fn frame(&self, dest: Rect, opacity: u8) {
        let (sw, sh) = self.source_size;
        self.place(0, RECT { left: 0, top: 0, right: sw, bottom: sh }, dest, opacity);
    }

    /// Places strip `i` of `strips()` (top to bottom) at `dest`.
    pub fn frame_strip(&self, i: usize, dest: Rect, opacity: u8) {
        let n = self.thumbs.len().max(1) as i32;
        let (sw, sh) = self.source_size;
        let i = i as i32;
        let src = RECT { left: 0, top: sh * i / n, right: sw, bottom: sh * (i + 1) / n };
        self.place(i as usize, src, dest, opacity);
    }

    fn place(&self, i: usize, src: RECT, dest: Rect, opacity: u8) {
        let Some(&thumb) = self.thumbs.get(i) else { return };
        let (ox, oy) = self.origin;
        let props = DWM_THUMBNAIL_PROPERTIES {
            dwFlags: DWM_TNP_RECTDESTINATION
                | DWM_TNP_RECTSOURCE
                | DWM_TNP_OPACITY
                | DWM_TNP_VISIBLE
                | DWM_TNP_SOURCECLIENTAREAONLY,
            rcDestination: RECT {
                left: dest.x - ox,
                top: dest.y - oy,
                right: dest.x - ox + dest.w.max(1),
                bottom: dest.y - oy + dest.h.max(1),
            },
            rcSource: src,
            opacity,
            fVisible: true.into(),
            fSourceClientAreaOnly: false.into(),
        };
        unsafe {
            let _ = DwmUpdateThumbnailProperties(thumb, &props);
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
        if !self.thumbs.is_empty() {
            unsafe {
                for t in self.thumbs.drain(..) {
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
