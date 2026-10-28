//! Tiny file logger: no background thread, no formatting crates.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

struct FileLogger {
    file: Mutex<Option<File>>,
    level: log::LevelFilter,
}

impl log::Log for FileLogger {
    fn enabled(&self, meta: &log::Metadata<'_>) -> bool {
        meta.level() <= self.level
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let line = format!("{secs} {:<5} {}: {}\n", record.level(), record.target(), record.args());
        if cfg!(debug_assertions) {
            eprint!("{line}");
        }
        if let Ok(mut f) = self.file.lock()
            && let Some(f) = f.as_mut()
        {
            let _ = f.write_all(line.as_bytes());
        }
    }

    fn flush(&self) {
        if let Ok(mut f) = self.file.lock()
            && let Some(f) = f.as_mut()
        {
            let _ = f.flush();
        }
    }
}

/// Logs to `path` (truncated when it grows past 1 MiB). `FAMILIAR_LOG`
/// sets the level (`error`..`trace`, default `info`).
pub fn init(path: &Path) {
    let too_big = std::fs::metadata(path).map(|m| m.len() > 1 << 20).unwrap_or(false);
    let file = OpenOptions::new().create(true).append(!too_big).write(true).truncate(too_big).open(path).ok();
    let level = std::env::var("FAMILIAR_LOG").ok().and_then(|l| l.parse().ok()).unwrap_or(log::LevelFilter::Info);
    let logger = Box::leak(Box::new(FileLogger { file: Mutex::new(file), level }));
    if log::set_logger(logger).is_ok() {
        log::set_max_level(level);
    }
}
