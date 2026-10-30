//! Opening the launcher with a bare tap of the Windows key.
//!
//! A `WH_KEYBOARD_LL` hook (no DLL injection: low-level hooks run in our own
//! process) watches for Win pressed and released with nothing in between.
//! On such a tap it injects an unassigned key (VK 0xE8) before the release
//! so the shell sees a "combo" and does not open the Start menu.

use std::cell::RefCell;
use std::sync::mpsc;
use std::thread::JoinHandle;

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, HHOOK, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG, PostThreadMessageW,
    SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_QUIT, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

const VK_LWIN: u32 = 0x5B;
const VK_RWIN: u32 = 0x5C;
const VK_MASK: u16 = 0xE8;

#[derive(Default)]
struct State {
    win_down: bool,
    combo: bool,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    static ON_TAP: RefCell<Option<Box<dyn Fn()>>> = RefCell::new(None);
}

pub struct WinKeyHook {
    thread_id: u32,
    join: Option<JoinHandle<()>>,
}

impl WinKeyHook {
    pub fn spawn(on_tap: impl Fn() + Send + 'static) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let join = std::thread::Builder::new().name("winkey-hook".into()).spawn(move || unsafe {
            ON_TAP.with(|t| *t.borrow_mut() = Some(Box::new(on_tap)));
            let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), None, 0);
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
            return Err(std::io::Error::other("SetWindowsHookExW(WH_KEYBOARD_LL) failed"));
        }
        Ok(Self { thread_id, join: Some(join) })
    }
}

impl Drop for WinKeyHook {
    fn drop(&mut self) {
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

fn send_mask_key() {
    let key = |flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: VIRTUAL_KEY(VK_MASK), wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 },
        },
    };
    let inputs = [key(KEYBD_EVENT_FLAGS(0)), key(KEYEVENTF_KEYUP)];
    unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
}

unsafe extern "system" fn hook_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code >= 0 {
        crate::safety::ffi_guard((), || on_key(wp, lp));
    }
    unsafe { CallNextHookEx(Some(HHOOK::default()), code, wp, lp) }
}

fn on_key(wp: WPARAM, lp: LPARAM) {
    {
        // SAFETY: for WH_KEYBOARD_LL, lParam points to a KBDLLHOOKSTRUCT.
        let k = unsafe { &*(lp.0 as *const KBDLLHOOKSTRUCT) };
        if k.flags.0 & LLKHF_INJECTED.0 == 0 {
            let is_win = k.vkCode == VK_LWIN || k.vkCode == VK_RWIN;
            let msg = wp.0 as u32;
            let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
            let up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
            let tapped = STATE.with(|s| {
                let mut s = s.borrow_mut();
                if is_win && down {
                    if !s.win_down {
                        s.win_down = true;
                        s.combo = false;
                    }
                } else if !is_win && down && s.win_down {
                    s.combo = true;
                } else if is_win && up && s.win_down {
                    s.win_down = false;
                    return !s.combo;
                }
                false
            });
            if tapped {
                send_mask_key();
                ON_TAP.with(|t| {
                    if let Some(f) = t.borrow().as_ref() {
                        f();
                    }
                });
            }
        }
    }
}
