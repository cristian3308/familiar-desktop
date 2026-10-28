//! Screen-space reservation via `SHAppBarMessage`.
//!
//! Registering the top bar as an AppBar makes Windows shrink the work area,
//! so maximised windows stop below the bar instead of under it. The shell
//! also tells AppBars when a full-screen app opens, which drives calm mode.

use std::cell::RefCell;
use std::rc::Rc;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::UI::Shell::{
    ABE_BOTTOM, ABE_TOP, ABM_ACTIVATE, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS, ABM_WINDOWPOSCHANGED,
    ABN_FULLSCREENAPP, ABN_POSCHANGED, APPBARDATA, SHAppBarMessage,
};
use windows::Win32::UI::WindowsAndMessaging::{WM_ACTIVATE, WM_APP, WM_WINDOWPOSCHANGED};

use crate::subclass;
use crate::window::{Rect, set_bounds};

pub const APPBAR_CALLBACK: u32 = WM_APP + 0x51;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Top,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppBarEvent {
    /// Another AppBar moved or the taskbar changed; we re-reserved and the
    /// bar now sits at this rectangle.
    Repositioned(Rect),
    /// A full-screen app opened (`true`) or closed (`false`) on our monitor.
    FullScreen(bool),
}

struct Inner {
    hwnd: HWND,
    edge: Edge,
    thickness: i32,
    monitor: Rect,
    /// Move the window onto the reserved strip (top bar) or leave its
    /// placement to the caller (the dock is narrower than the strip).
    move_window: bool,
    registered: bool,
}

pub struct AppBar {
    inner: Rc<RefCell<Inner>>,
}

fn abd(hwnd: HWND) -> APPBARDATA {
    APPBARDATA { cbSize: std::mem::size_of::<APPBARDATA>() as u32, hWnd: hwnd, ..Default::default() }
}

impl AppBar {
    /// Registers `hwnd` and reserves `thickness` physical pixels along
    /// `edge` of `monitor`. Must be called on the window's thread.
    pub fn register(
        hwnd: HWND,
        edge: Edge,
        thickness: i32,
        monitor: Rect,
        move_window: bool,
        on_event: impl Fn(AppBarEvent) + 'static,
    ) -> Option<(Self, Rect)> {
        let mut data = abd(hwnd);
        data.uCallbackMessage = APPBAR_CALLBACK;
        if unsafe { SHAppBarMessage(ABM_NEW, &mut data) } == 0 {
            return None;
        }
        let inner = Rc::new(RefCell::new(Inner { hwnd, edge, thickness, monitor, move_window, registered: true }));
        let rect = reserve(&inner);

        let weak = Rc::downgrade(&inner);
        subclass::add(
            hwnd,
            Rc::new(move |hwnd, msg, wp: WPARAM, lp: LPARAM| match msg {
                APPBAR_CALLBACK => {
                    let inner = weak.upgrade()?;
                    match wp.0 as u32 {
                        ABN_POSCHANGED => {
                            let r = reserve(&inner);
                            on_event(AppBarEvent::Repositioned(r));
                        }
                        ABN_FULLSCREENAPP => on_event(AppBarEvent::FullScreen(lp.0 != 0)),
                        _ => {}
                    }
                    Some(LRESULT(0))
                }
                WM_ACTIVATE => {
                    let mut d = abd(hwnd);
                    unsafe { SHAppBarMessage(ABM_ACTIVATE, &mut d) };
                    None
                }
                WM_WINDOWPOSCHANGED => {
                    let mut d = abd(hwnd);
                    unsafe { SHAppBarMessage(ABM_WINDOWPOSCHANGED, &mut d) };
                    None
                }
                _ => None,
            }),
        );
        Some((Self { inner }, rect))
    }

    /// Changes the reserved thickness (theme change) and returns the new rect.
    pub fn set_thickness(&self, thickness: i32) -> Rect {
        self.inner.borrow_mut().thickness = thickness;
        reserve(&self.inner)
    }

    pub fn set_monitor(&self, monitor: Rect) -> Rect {
        self.inner.borrow_mut().monitor = monitor;
        reserve(&self.inner)
    }

    pub fn unregister(&self) {
        let mut inner = self.inner.borrow_mut();
        if inner.registered {
            let mut d = abd(inner.hwnd);
            unsafe { SHAppBarMessage(ABM_REMOVE, &mut d) };
            inner.registered = false;
        }
    }
}

impl Drop for AppBar {
    fn drop(&mut self) {
        self.unregister();
    }
}

/// ABM_QUERYPOS → adjust → ABM_SETPOS, then moves the window there.
fn reserve(inner: &Rc<RefCell<Inner>>) -> Rect {
    let (hwnd, edge, thickness, mon, move_window) = {
        let i = inner.borrow();
        (i.hwnd, i.edge, i.thickness, i.monitor, i.move_window)
    };
    let mut d = abd(hwnd);
    d.uEdge = match edge {
        Edge::Top => ABE_TOP,
        Edge::Bottom => ABE_BOTTOM,
    };
    d.rc = RECT { left: mon.x, top: mon.y, right: mon.x + mon.w, bottom: mon.y + mon.h };
    match edge {
        Edge::Top => d.rc.bottom = d.rc.top + thickness,
        Edge::Bottom => d.rc.top = d.rc.bottom - thickness,
    }
    unsafe { SHAppBarMessage(ABM_QUERYPOS, &mut d) };
    // The shell may have moved the proposed edge; re-apply our thickness.
    match edge {
        Edge::Top => d.rc.bottom = d.rc.top + thickness,
        Edge::Bottom => d.rc.top = d.rc.bottom - thickness,
    }
    unsafe { SHAppBarMessage(ABM_SETPOS, &mut d) };
    let r = Rect::from_win(d.rc);
    if move_window {
        set_bounds(hwnd, r);
    }
    r
}
