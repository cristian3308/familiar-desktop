//! Background work that touches the shell namespace: the installed-app
//! index and icon extraction. Results are posted back to the UI thread.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};

use familiar_store::AppEntry;
use familiar_sys::apps;
use familiar_sys::icons::{self, IconSource};

use crate::app::post;
use crate::config::{Paths, stable_hash};

const ICON_SIZE: u32 = 256;

pub fn icon_path(icon_dir: &Path, key: &str) -> PathBuf {
    icon_dir.join(format!("{:016x}.png", stable_hash(key)))
}

/// Enumerates installed apps and caches any missing icons, then hands the
/// list to [`crate::app::App::on_index_ready`].
pub fn spawn_index(paths: &Paths) {
    let icon_dir = paths.icon_dir();
    let spawned = std::thread::Builder::new().name("app-index".into()).spawn(move || {
        let started = std::time::Instant::now();
        let installed = match apps::installed_apps() {
            Ok(a) => a,
            Err(e) => {
                log::error!("enumerating AppsFolder failed: {e}");
                return;
            }
        };
        let mut extracted = 0;
        let entries: Vec<AppEntry> = installed
            .into_iter()
            .map(|a| {
                let icon = icon_path(&icon_dir, &a.parsing_name);
                if !icon.exists() && icons::extract_png(IconSource::AppsFolder(&a.parsing_name), ICON_SIZE, &icon) {
                    extracted += 1;
                }
                AppEntry {
                    name: a.name,
                    launch_target: a.parsing_name,
                    executable_path: a.exe_path,
                    icon_cache_path: if icon.exists() { icon.to_string_lossy().into_owned() } else { String::new() },
                    category: Some(if a.packaged { "Microsoft Store".into() } else { "Aplicación".into() }),
                    ..Default::default()
                }
            })
            .collect();
        log::info!("indexed {} apps ({extracted} new icons) in {:?}", entries.len(), started.elapsed());
        post(move |app| app.on_index_ready(entries));
    });
    if let Err(e) = spawned {
        log::error!("cannot start indexer: {e}");
    }
}

struct IconJob {
    key: String,
    exe: String,
    out: PathBuf,
}

/// Serial icon extraction for apps discovered at runtime (running windows
/// that are not in the index yet).
pub struct IconWorker {
    tx: Sender<IconJob>,
    icon_dir: PathBuf,
}

impl IconWorker {
    pub fn spawn(paths: &Paths) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel::<IconJob>();
        std::thread::Builder::new().name("icons".into()).spawn(move || {
            while let Ok(job) = rx.recv() {
                let ok = if !job.exe.is_empty() {
                    icons::extract_png(IconSource::Path(&job.exe), ICON_SIZE, &job.out)
                } else {
                    false
                } || icons::extract_png(IconSource::AppsFolder(&job.key), ICON_SIZE, &job.out);
                if ok {
                    post(move |app| app.on_icon_ready(&job.key, &job.out));
                }
            }
        })?;
        Ok(Self { tx, icon_dir: paths.icon_dir() })
    }

    pub fn path_for(&self, key: &str) -> PathBuf {
        icon_path(&self.icon_dir, key)
    }

    pub fn request(&self, key: &str, exe: &str) {
        let _ = self.tx.send(IconJob { key: key.to_string(), exe: exe.to_string(), out: self.path_for(key) });
    }
}
