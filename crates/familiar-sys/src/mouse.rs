//! Cursor effects input: global clicks (ripple) and shake-to-find.
//! Installed only when the theme enables one of the effects.

use std::cell::RefCell;
use std::sync::mpsc;
use std::thread::JoinHandle;

use familiar_layout::ShakeDetector;
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, HHOOK, MSG, MSLLHOOKSTRUCT, PostThreadMessageW, SetWindowsHookExW,
    UnhookWindowsHookEx, WH_MOUSE_LL, WM_LBUTTONDOWN, WM_MOUSEMOVE, WM_QUIT, WM_RBUTTONDOWN,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseFx {
    Click { x: i32, y: i32 },
    Shake { x: i32, y: i32 },
}

struct Config {
    clicks: bool,
    shake: bool,
}

type HookState = (Config, ShakeDetector, Box<dyn Fn(MouseFx)>);

thread_local! {
    static CB: RefCell<Option<HookState>> = RefCell::new(None);
}

pub struct MouseHook {
    thread_id: u32,
    join: Option<JoinHandle<()>>,
}

impl MouseHook {
    pub fn spawn(clicks: bool, shake: bool, cb: impl Fn(MouseFx) + Send + 'static) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let join = std::thread::Builder::new().name("mouse-fx".into()).spawn(move || unsafe {
            CB.with(|c| *c.borrow_mut() = Some((Config { clicks, shake }, ShakeDetector::default(), Box::new(cb))));
            let hook = SetWindowsHookExW(WH_MOUSE_LL, Some(hook_proc), None, 0);
            let _ = tx.send((GetCurrentThreadId(), hook.is_ok()));
            let Ok(hook) = hook else { return };
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                DispatchMessageW(&msg);
            }
            let _ = UnhookWindowsHookEx(hook);
        })?;
        let (thread_id, ok) = rx.recv().map_err(|_| std::io::Error::other("hook thread died"))?;
        if !ok {
            let _ = join.join();
            return Err(std::io::Error::other("SetWindowsHookExW(WH_MOUSE_LL) failed"));
        }
        Ok(Self { thread_id, join: Some(join) })
    }
}

impl Drop for MouseHook {
    fn drop(&mut self) {
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

unsafe extern "system" fn hook_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code >= 0 {
        crate::safety::ffi_guard((), || on_mouse(wp, lp));
    }
    // Hooks must return quickly or Windows silently removes them.
    unsafe { CallNextHookEx(Some(HHOOK::default()), code, wp, lp) }
}

fn on_mouse(wp: WPARAM, lp: LPARAM) {
    {
        // SAFETY: for WH_MOUSE_LL, lParam points to an MSLLHOOKSTRUCT.
        let m = unsafe { &*(lp.0 as *const MSLLHOOKSTRUCT) };
        let (x, y) = (m.pt.x, m.pt.y);
        CB.with(|c| {
            if let Some((cfg, shake, cb)) = c.borrow_mut().as_mut() {
                match wp.0 as u32 {
                    WM_LBUTTONDOWN | WM_RBUTTONDOWN if cfg.clicks => cb(MouseFx::Click { x, y }),
                    WM_MOUSEMOVE if cfg.shake && shake.push(x as f32, m.time) => {
                        cb(MouseFx::Shake { x, y });
                    }
                    _ => {}
                }
            }
        });
    }
}
