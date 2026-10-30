//! CPU and memory load for the system widget.

use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::GetSystemTimes;

fn ticks(f: FILETIME) -> u64 {
    (u64::from(f.dwHighDateTime) << 32) | u64::from(f.dwLowDateTime)
}

/// Samples overall CPU usage between two calls.
#[derive(Default)]
pub struct CpuMeter {
    last: Option<(u64, u64)>,
}

impl CpuMeter {
    /// Percentage busy since the previous call (0 on the first call).
    pub fn sample(&mut self) -> u32 {
        let (mut idle, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
        if unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }.is_err() {
            return 0;
        }
        // Kernel time includes idle time.
        let (idle, total) = (ticks(idle), ticks(kernel) + ticks(user));
        let pct = match self.last {
            Some((li, lt)) if total > lt => {
                let busy = (total - lt).saturating_sub(idle.saturating_sub(li));
                (busy * 100 / (total - lt)) as u32
            }
            _ => 0,
        };
        self.last = Some((idle, total));
        pct.min(100)
    }
}

/// Physical memory in use, in percent, plus used and total GiB.
pub fn memory() -> (u32, f32, f32) {
    let mut m = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    if unsafe { GlobalMemoryStatusEx(&mut m) }.is_err() {
        return (0, 0.0, 0.0);
    }
    const GIB: f32 = 1024.0 * 1024.0 * 1024.0;
    let total = m.ullTotalPhys as f32 / GIB;
    let used = (m.ullTotalPhys - m.ullAvailPhys) as f32 / GIB;
    (m.dwMemoryLoad, used, total)
}
