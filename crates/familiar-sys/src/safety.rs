//! Crash safety: whatever happens, give the user their taskbar back.
//!
//! Win32 calls our hooks and window procedures through the `system` ABI; a
//! Rust panic unwinding across that boundary is undefined behaviour. Every
//! such callback runs its body inside [`ffi_guard`], which stops the unwind
//! and returns a safe fallback instead.

use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::System::Console::SetConsoleCtrlHandler;
use windows::core::BOOL;

type RestoreFn = Box<dyn Fn() + Send + Sync>;

static RESTORE: OnceLock<RestoreFn> = OnceLock::new();
static RESTORED: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// Depth of [`ffi_guard`] calls on this thread. A panic caught there is
    /// recovered from, so it must not tear the desktop down.
    static GUARDED: Cell<u32> = const { Cell::new(0) };
}

/// Runs `f`, the body of a Win32 callback, so that a panic never unwinds
/// into the operating system: it is logged and `fallback` is returned.
pub fn ffi_guard<R>(fallback: R, f: impl FnOnce() -> R) -> R {
    GUARDED.with(|g| g.set(g.get() + 1));
    let result = catch_unwind(AssertUnwindSafe(f));
    GUARDED.with(|g| g.set(g.get() - 1));
    result.unwrap_or_else(|_| {
        log::error!("panic caught at an FFI boundary; callback skipped");
        fallback
    })
}

fn guarded() -> bool {
    GUARDED.with(|g| g.get() > 0)
}

/// Registers `restore` to run on panic, console close/Ctrl+C, and through
/// [`run_restore`]. Only the first call has an effect.
pub fn install(restore: impl Fn() + Send + Sync + 'static) {
    if RESTORE.set(Box::new(restore)).is_err() {
        return;
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // A panic inside ffi_guard is recovered from; keep the desktop.
        if !guarded() {
            run_restore();
        }
        previous(info);
    }));
    unsafe {
        let _ = SetConsoleCtrlHandler(Some(console_handler), true);
    }
}

/// Runs the restore callback once; later calls are no-ops.
pub fn run_restore() {
    if RESTORED.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Some(f) = RESTORE.get() {
        // Never let a failure here turn into a double panic.
        let _ = catch_unwind(AssertUnwindSafe(f));
    }
}

unsafe extern "system" fn console_handler(_ctrl: u32) -> BOOL {
    ffi_guard((), run_restore);
    // Let the default handler terminate the process.
    false.into()
}

#[cfg(test)]
mod tests {
    use super::ffi_guard;

    #[test]
    fn guard_returns_value() {
        assert_eq!(ffi_guard(0, || 7), 7);
    }

    #[test]
    fn guard_stops_panics() {
        let r = ffi_guard(-1, || -> i32 { panic!("boom") });
        assert_eq!(r, -1);
        // The depth counter is balanced again afterwards.
        assert!(!super::guarded());
    }
}
