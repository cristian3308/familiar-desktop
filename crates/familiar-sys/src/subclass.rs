//! Minimal window subclassing for windows owned by the UI toolkit.
//!
//! Slint/winit own the window procedure; we chain in front of it to see a
//! few messages (AppBar callbacks, `WM_WINDOWPOSCHANGING`). Handlers are
//! `Rc` so they are cloned out of the registry before being called, which
//! keeps re-entrant messages (very common with shell APIs) from panicking.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, GWLP_WNDPROC, GetWindowLongPtrW, SetWindowLongPtrW, WM_NCDESTROY, WNDPROC,
};

use crate::Hwnd;

/// Return `Some(result)` to swallow the message, `None` to pass it on.
pub type Handler = Rc<dyn Fn(HWND, u32, WPARAM, LPARAM) -> Option<LRESULT>>;

struct Entry {
    original: isize,
    handlers: Vec<Handler>,
}

thread_local! {
    static REGISTRY: RefCell<HashMap<Hwnd, Entry>> = RefCell::new(HashMap::new());
}

/// Adds `handler` in front of the window procedure of `hwnd`.
/// Must be called on the thread that owns the window.
pub fn add(hwnd: HWND, handler: Handler) {
    REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        if let Some(e) = r.get_mut(&hwnd.into()) {
            e.handlers.push(handler);
            return;
        }
        let original = unsafe { GetWindowLongPtrW(hwnd, GWLP_WNDPROC) };
        unsafe { SetWindowLongPtrW(hwnd, GWLP_WNDPROC, subclass_proc as *const () as isize) };
        r.insert(hwnd.into(), Entry { original, handlers: vec![handler] });
    });
}

unsafe extern "system" fn subclass_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let found = REGISTRY.with(|r| r.borrow().get(&hwnd.into()).map(|e| (e.original, e.handlers.clone())));
    let Some((original, handlers)) = found else {
        return LRESULT(0);
    };
    for h in &handlers {
        if let Some(res) = h(hwnd, msg, wp, lp) {
            return res;
        }
    }
    if msg == WM_NCDESTROY {
        REGISTRY.with(|r| r.borrow_mut().remove(&hwnd.into()));
        unsafe { SetWindowLongPtrW(hwnd, GWLP_WNDPROC, original) };
    }
    // SAFETY: `original` was read from GWLP_WNDPROC of this window.
    let proc: WNDPROC = unsafe { std::mem::transmute::<isize, WNDPROC>(original) };
    unsafe { CallWindowProcW(proc, hwnd, msg, wp, lp) }
}
