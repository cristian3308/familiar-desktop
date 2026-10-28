//! Hot reload: watches the theme file and calls back with the parsed theme.
//!
//! `notify` uses `ReadDirectoryChangesW` on Windows, so an idle watcher costs
//! no CPU. The parent directory is watched because editors usually save by
//! writing a temp file and renaming it over the original.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use crate::Theme;

pub struct ThemeWatcher {
    _watcher: RecommendedWatcher,
}

impl ThemeWatcher {
    /// `on_change` runs on a background thread after each successful reload.
    /// Parse errors are logged and the previous theme stays active.
    pub fn spawn(path: &Path, on_change: impl Fn(Theme) + Send + 'static) -> notify::Result<Self> {
        let path = path.to_path_buf();
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
        let (tx, rx) = mpsc::channel::<()>();
        let target = path.clone();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res
                && ev.paths.iter().any(|p| same_file(p, &target))
            {
                let _ = tx.send(());
            }
        })?;
        watcher.watch(&dir, RecursiveMode::NonRecursive)?;

        std::thread::Builder::new()
            .name("theme-watch".into())
            .spawn(move || {
                // Blocks with zero CPU until the first event; then coalesces
                // the burst of events a single save produces.
                while rx.recv().is_ok() {
                    while rx.recv_timeout(Duration::from_millis(120)).is_ok() {}
                    match Theme::load(&path) {
                        Ok(theme) => on_change(theme),
                        Err(e) => log::warn!("theme reload skipped: {e}"),
                    }
                }
            })
            .map_err(notify::Error::io)?;
        Ok(Self { _watcher: watcher })
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    a.file_name() == b.file_name()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reloads_on_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("theme.toml");
        std::fs::write(&path, Theme::default().to_toml().unwrap()).unwrap();
        let (tx, rx) = mpsc::channel();
        let _w = ThemeWatcher::spawn(&path, move |t| {
            let _ = tx.send(t);
        })
        .unwrap();
        std::thread::sleep(Duration::from_millis(200));
        std::fs::write(&path, "[meta]\nname = \"Changed\"\n").unwrap();
        let t = rx.recv_timeout(Duration::from_secs(5)).expect("reload event");
        assert_eq!(t.meta.name, "Changed");
    }
}
