use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize,
};

/// Initialises COM for the current thread and uninitialises it on drop.
pub struct ComGuard {
    owned: bool,
}

impl ComGuard {
    /// Single-threaded apartment, required by most Shell APIs.
    pub fn sta() -> Self {
        Self::init(COINIT_APARTMENTTHREADED.0 | COINIT_DISABLE_OLE1DDE.0)
    }

    /// Multi-threaded apartment, used by WASAPI and WinRT worker threads.
    pub fn mta() -> Self {
        Self::init(COINIT_MULTITHREADED.0)
    }

    fn init(flags: i32) -> Self {
        let hr = unsafe { CoInitializeEx(None, windows::Win32::System::Com::COINIT(flags)) };
        // S_FALSE (already initialised) still needs a matching uninit;
        // RPC_E_CHANGED_MODE means someone else owns the apartment.
        Self { owned: hr.is_ok() }
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.owned {
            unsafe { CoUninitialize() };
        }
    }
}
