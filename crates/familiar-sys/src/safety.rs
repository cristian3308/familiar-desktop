//! Crash safety: whatever happens, give the user their taskbar back.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::System::Console::SetConsoleCtrlHandler;
use windows::core::BOOL;

type RestoreFn = Box<dyn Fn() + Send + Sync>;

static RESTORE: OnceLock<RestoreFn> = OnceLock::new();
static RESTORED: AtomicBool = AtomicBool::new(false);

/// Registers `restore` to run on panic, console close/Ctrl+C, and through
/// [`run_restore`]. Only the first call has an effect.
pub fn install(restore: impl Fn() + Send + Sync + 'static) {
    if RESTORE.set(Box::new(restore)).is_err() {
        return;
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        run_restore();
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
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    }
}

unsafe extern "system" fn console_handler(_ctrl: u32) -> BOOL {
    run_restore();
    // Let the default handler terminate the process.
    false.into()
}
