//! The shell watcher thread.
//!
//! One thread owns a hidden top-level window plus the out-of-context
//! WinEvent hooks, and sleeps in `GetMessageW` until Windows has something
//! to say. It reports foreground switches, window churn, hotkeys, Explorer
//! restarts and session/power/display changes.

use std::cell::RefCell;
use std::sync::mpsc;
use std::thread::JoinHandle;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey, UnregisterHotKey,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CHILDID_SELF, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, EVENT_OBJECT_CLOAKED,
    EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE, EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_SHOW,
    EVENT_OBJECT_UNCLOAKED, EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND, EVENT_SYSTEM_MINIMIZESTART, GetMessageW,
    MSG, OBJID_WINDOW, PostThreadMessageW, RegisterClassW, RegisterWindowMessageW, TranslateMessage,
    WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_DISPLAYCHANGE, WM_ENDSESSION, WM_HOTKEY, WM_POWERBROADCAST,
    WM_QUERYENDSESSION, WM_QUIT, WM_SETTINGCHANGE, WNDCLASSW, WS_EX_TOOLWINDOW, WS_OVERLAPPED,
};
use windows::core::w;

use crate::Hwnd;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellEvent {
    /// The foreground window changed.
    Foreground(Hwnd),
    /// A top-level window appeared, disappeared, (un)minimised or cloaked;
    /// consumers should re-enumerate (debounced).
    WindowsChanged,
    /// A window title changed.
    TitleChanged(Hwnd),
    /// An app window appeared (shown or uncloaked); drives the open effect.
    WindowShown(Hwnd),
    /// An app window started minimising / was restored.
    MinimizeStart(Hwnd),
    MinimizeEnd(Hwnd),
    /// Ctrl+Alt+Q: restore the taskbar and quit.
    EmergencyExit,
    /// The launcher hotkey (when registered as a classic hotkey).
    LauncherHotkey,
    /// Explorer restarted; AppBars and taskbar hiding must be redone.
    ExplorerRestarted,
    /// Windows is logging off or shutting down.
    SessionEnding,
    /// Resolution, DPI, monitor layout or work area changed.
    DisplayChanged,
    /// AC/battery or battery-saver state changed.
    PowerChanged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hotkey {
    pub modifiers: u32,
    pub vk: u32,
}

impl Hotkey {
    /// Parses strings like `"alt+space"` or `"ctrl+shift+k"`.
    pub fn parse(s: &str) -> Option<Self> {
        let mut modifiers = 0u32;
        let mut vk = None;
        for part in s.split('+').map(|p| p.trim().to_ascii_lowercase()) {
            match part.as_str() {
                "ctrl" | "control" => modifiers |= MOD_CONTROL.0,
                "alt" => modifiers |= MOD_ALT.0,
                "shift" => modifiers |= MOD_SHIFT.0,
                "win" => modifiers |= MOD_WIN.0,
                "space" => vk = Some(0x20),
                "enter" => vk = Some(0x0D),
                "tab" => vk = Some(0x09),
                k if k.len() == 1 && k.as_bytes()[0].is_ascii_alphanumeric() => {
                    vk = Some(k.as_bytes()[0].to_ascii_uppercase() as u32)
                }
                k if k.starts_with('f') && k[1..].parse::<u32>().is_ok_and(|n| (1..=24).contains(&n)) => {
                    vk = Some(0x70 + k[1..].parse::<u32>().unwrap() - 1)
                }
                _ => return None,
            }
        }
        Some(Self { modifiers, vk: vk? })
    }
}

pub struct ShellWatcherOptions {
    pub launcher_hotkey: Option<Hotkey>,
}

pub struct ShellWatcher {
    thread_id: u32,
    join: Option<JoinHandle<()>>,
}

type Callback = Box<dyn Fn(ShellEvent) + Send>;

thread_local! {
    static CALLBACK: RefCell<Option<Callback>> = const { RefCell::new(None) };
    static TASKBAR_CREATED: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

fn emit(ev: ShellEvent) {
    CALLBACK.with(|c| {
        if let Some(cb) = c.borrow().as_ref() {
            cb(ev);
        }
    });
}

const HOTKEY_EMERGENCY: i32 = 1;
const HOTKEY_LAUNCHER: i32 = 2;

impl ShellWatcher {
    pub fn spawn(opts: ShellWatcherOptions, cb: impl Fn(ShellEvent) + Send + 'static) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let join = std::thread::Builder::new().name("shell-watch".into()).spawn(move || {
            CALLBACK.with(|c| *c.borrow_mut() = Some(Box::new(cb)));
            let _ = tx.send(unsafe { GetCurrentThreadId() });
            unsafe { run(opts) };
        })?;
        let thread_id = rx.recv().map_err(|_| std::io::Error::other("shell watcher died"))?;
        Ok(Self { thread_id, join: Some(join) })
    }
}

impl Drop for ShellWatcher {
    fn drop(&mut self) {
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

unsafe fn run(opts: ShellWatcherOptions) {
    let hinstance = unsafe { GetModuleHandleW(None) }.unwrap_or_default();
    let class = WNDCLASSW {
        lpfnWndProc: Some(wnd_proc),
        hInstance: hinstance.into(),
        lpszClassName: w!("FamiliarCoreShellWatcher"),
        ..Default::default()
    };
    unsafe { RegisterClassW(&class) };
    TASKBAR_CREATED.with(|t| t.set(unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) }));
    // A real (hidden) top-level window, not HWND_MESSAGE: message-only
    // windows do not receive broadcasts such as TaskbarCreated or
    // WM_ENDSESSION.
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            w!("FamiliarCoreShellWatcher"),
            w!("Familiar Core"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinstance.into()),
            None,
        )
    };
    let hwnd = match hwnd {
        Ok(h) => h,
        Err(e) => {
            log::error!("shell watcher window: {e}");
            return;
        }
    };

    unsafe {
        if RegisterHotKey(Some(hwnd), HOTKEY_EMERGENCY, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, 'Q' as u32).is_err() {
            log::warn!("Ctrl+Alt+Q is taken by another app; emergency exit hotkey unavailable");
        }
        if let Some(hk) = opts.launcher_hotkey {
            let mods = HOT_KEY_MODIFIERS(hk.modifiers) | MOD_NOREPEAT;
            if RegisterHotKey(Some(hwnd), HOTKEY_LAUNCHER, mods, hk.vk).is_err() {
                log::warn!("launcher hotkey is taken by another app");
            }
        }
    }

    let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
    let ranges = [
        (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND),
        (EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZEEND),
        (EVENT_OBJECT_CREATE, EVENT_OBJECT_HIDE),
        (EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_NAMECHANGE),
        (EVENT_OBJECT_CLOAKED, EVENT_OBJECT_UNCLOAKED),
    ];
    let hooks: Vec<HWINEVENTHOOK> = ranges
        .iter()
        .map(|(lo, hi)| unsafe { SetWinEventHook(*lo, *hi, None, Some(win_event_proc), 0, 0, flags) })
        .filter(|h| !h.is_invalid())
        .collect();

    let mut msg = MSG::default();
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    unsafe {
        for h in hooks {
            let _ = UnhookWinEvent(h);
        }
        let _ = UnregisterHotKey(Some(hwnd), HOTKEY_EMERGENCY);
        let _ = UnregisterHotKey(Some(hwnd), HOTKEY_LAUNCHER);
        let _ = DestroyWindow(hwnd);
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let taskbar_created = TASKBAR_CREATED.with(|t| t.get());
    match msg {
        WM_HOTKEY => match wp.0 as i32 {
            HOTKEY_EMERGENCY => emit(ShellEvent::EmergencyExit),
            HOTKEY_LAUNCHER => emit(ShellEvent::LauncherHotkey),
            _ => {}
        },
        WM_QUERYENDSESSION => return LRESULT(1),
        WM_ENDSESSION if wp.0 != 0 => emit(ShellEvent::SessionEnding),
        WM_DISPLAYCHANGE => emit(ShellEvent::DisplayChanged),
        // SPI_SETWORKAREA (0x2F) is sent when an AppBar or the taskbar moves.
        WM_SETTINGCHANGE if wp.0 == 0x2F => emit(ShellEvent::DisplayChanged),
        WM_POWERBROADCAST => emit(ShellEvent::PowerChanged),
        m if m != 0 && m == taskbar_created => emit(ShellEvent::ExplorerRestarted),
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

unsafe extern "system" fn win_event_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _thread: u32,
    _time: u32,
) {
    if id_object != OBJID_WINDOW.0 || id_child != CHILDID_SELF as i32 || hwnd.is_invalid() {
        return;
    }
    match event {
        EVENT_SYSTEM_FOREGROUND => emit(ShellEvent::Foreground(hwnd.into())),
        EVENT_OBJECT_NAMECHANGE => {
            if crate::windows_list::is_top_level(hwnd) {
                emit(ShellEvent::TitleChanged(hwnd.into()));
            }
        }
        EVENT_OBJECT_DESTROY => emit(ShellEvent::WindowsChanged),
        EVENT_OBJECT_SHOW | EVENT_OBJECT_UNCLOAKED if crate::windows_list::is_app_window(hwnd) => {
            emit(ShellEvent::WindowShown(hwnd.into()));
            emit(ShellEvent::WindowsChanged);
        }
        EVENT_SYSTEM_MINIMIZESTART => {
            emit(ShellEvent::MinimizeStart(hwnd.into()));
            emit(ShellEvent::WindowsChanged);
        }
        EVENT_SYSTEM_MINIMIZEEND => {
            emit(ShellEvent::MinimizeEnd(hwnd.into()));
            emit(ShellEvent::WindowsChanged);
        }
        EVENT_OBJECT_CREATE | EVENT_OBJECT_SHOW | EVENT_OBJECT_HIDE | EVENT_OBJECT_CLOAKED | EVENT_OBJECT_UNCLOAKED
            if crate::windows_list::is_top_level(hwnd) =>
        {
            emit(ShellEvent::WindowsChanged);
        }
        _ => {}
    }
}
