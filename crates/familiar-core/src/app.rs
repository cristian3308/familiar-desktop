//! The single-process shell: owns every surface and every watcher.
//!
//! All UI state lives on the main thread. Watcher threads never touch it;
//! they [`post`] closures that run on the event loop with access to the
//! [`App`] stored in a thread-local.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Mutex;
use std::time::Duration;

use familiar_store::{AppEntry, NewDockItem, Store};
use familiar_sys::media::{MediaCommand, MediaService, NowPlaying};
use familiar_sys::mouse::MouseHook;
use familiar_sys::power::{self, Battery};
use familiar_sys::privacy::{PrivacyState, PrivacyWatcher};
use familiar_sys::shell::{Hotkey, ShellEvent, ShellWatcher, ShellWatcherOptions};
use familiar_sys::taskbar::{self, TaskbarState};
use familiar_sys::window::{MonitorInfo, primary_monitor};
use familiar_sys::windows_list::{self, AppWindow};
use familiar_sys::{Hwnd, apps, clock, cursor, keyboard::WinKeyHook};
use familiar_theme::{Theme as ThemeSpec, ThemeWatcher};
use slint::{Timer, TimerMode};

use crate::audio_pipe::AudioPipe;
use crate::config::{AppConfig, Paths};
use crate::controlcenter::ControlCenter;
use crate::dock::Dock;
use crate::fx::{CursorFx, WindowFx};
use crate::indexer::{self, IconWorker};
use crate::island::Island;
use crate::launcher::Launcher;
use crate::lock::Lock;
use crate::settings::Settings;
use crate::topbar::TopBar;
use crate::wallfx::WallpaperFx;
use crate::widgets::Widgets;
use crate::{logger, ui};

thread_local! {
    static APP: RefCell<Option<Rc<App>>> = const { RefCell::new(None) };
}

/// Runs `f` with the app if it exists (it does for the whole event loop).
pub fn with_app(f: impl FnOnce(&App)) {
    let app = APP.with(|a| a.borrow().clone());
    if let Some(app) = app {
        f(&app);
    }
}

/// Runs `f` on the UI thread, from any thread.
pub fn post(f: impl FnOnce(&App) + Send + 'static) {
    let _ = slint::invoke_from_event_loop(move || with_app(f));
}

/// Taskbar state needed to undo our changes from a panic hook or console
/// handler, i.e. without the UI thread.
static RESTORE: Mutex<Option<(TaskbarState, PathBuf)>> = Mutex::new(None);

fn restore_everything() {
    if let Ok(mut guard) = RESTORE.lock()
        && let Some((state, marker)) = guard.take()
    {
        taskbar::restore(state);
        let _ = std::fs::remove_file(marker);
    }
    cursor::restore_system();
}

#[derive(Default)]
struct Services {
    shell: Option<ShellWatcher>,
    privacy: Option<PrivacyWatcher>,
    media: Option<MediaService>,
    win_key: Option<WinKeyHook>,
    mouse: Option<MouseHook>,
    theme_watch: Option<ThemeWatcher>,
    icons: Option<IconWorker>,
}

pub struct App {
    pub cfg: RefCell<AppConfig>,
    pub paths: Paths,
    pub store: RefCell<Store>,
    pub theme: RefCell<ThemeSpec>,
    theme_path: RefCell<PathBuf>,
    pub monitor: Cell<MonitorInfo>,

    pub topbar: Option<TopBar>,
    pub dock: Option<Dock>,
    pub island: Option<Island>,
    pub widgets: Option<Widgets>,
    pub launcher: Launcher,
    pub settings: Settings,
    pub control_center: ControlCenter,
    pub lock: Lock,
    pub fx: RefCell<Option<CursorFx>>,
    pub window_fx: WindowFx,
    pub wallpaper_fx: WallpaperFx,
    pub audio: AudioPipe,

    services: RefCell<Services>,
    media: RefCell<Option<NowPlaying>>,
    privacy: RefCell<PrivacyState>,
    battery: Cell<Battery>,
    calm: Cell<bool>,
    custom_cursors: Cell<bool>,
    requested_icons: RefCell<std::collections::HashSet<String>>,
    windows_debounce: Timer,
    minute_align: Timer,
    minute: Timer,
}

impl App {
    fn new(
        cfg: AppConfig,
        paths: Paths,
        store: Store,
        theme: ThemeSpec,
        theme_path: PathBuf,
    ) -> Result<Self, slint::PlatformError> {
        let topbar = cfg.top_bar.enabled.then(TopBar::new).transpose()?;
        let dock = cfg.dock.enabled.then(Dock::new).transpose()?;
        let island = cfg.island.enabled.then(Island::new).transpose()?;
        // Always present (maybe empty) so Settings can add widgets live.
        let ids: &[String] = if cfg.widgets.enabled { &cfg.widgets.items } else { &[] };
        let widgets = Some(Widgets::new(ids)?);
        Ok(Self {
            launcher: Launcher::new()?,
            settings: Settings::new()?,
            control_center: ControlCenter::new()?,
            lock: Lock::new()?,
            topbar,
            dock,
            island,
            widgets,
            fx: RefCell::new(None),
            window_fx: WindowFx::new(),
            wallpaper_fx: WallpaperFx::new()?,
            audio: AudioPipe::new(),
            monitor: Cell::new(primary_monitor()),
            cfg: RefCell::new(cfg),
            paths,
            store: RefCell::new(store),
            theme: RefCell::new(theme),
            theme_path: RefCell::new(theme_path),
            services: RefCell::new(Services::default()),
            media: RefCell::new(None),
            privacy: RefCell::new(PrivacyState::default()),
            battery: Cell::new(Battery::default()),
            calm: Cell::new(false),
            custom_cursors: Cell::new(false),
            requested_icons: RefCell::new(Default::default()),
            windows_debounce: Timer::default(),
            minute_align: Timer::default(),
            minute: Timer::default(),
        })
    }

    fn start(&self) {
        if let Ok(apps) = self.store.borrow().apps() {
            self.launcher.set_apps(apps);
        }
        self.push_theme_everywhere();

        if self.cfg.borrow().hide_taskbar {
            self.hide_taskbar();
        }
        if let Some(t) = &self.topbar {
            t.show(self);
        }
        if let Some(d) = &self.dock {
            d.show(self);
        }
        // Our AppBars changed the work area.
        self.monitor.set(primary_monitor());
        if let Some(w) = &self.widgets {
            w.show(self);
        }
        if let Some(i) = &self.island {
            i.show(self);
        }

        self.start_services();
        self.battery.set(power::battery());
        if let Some(t) = &self.topbar {
            t.set_battery(self.battery.get());
            t.set_layout_code(&power::keyboard_layout_code());
        }
        self.on_foreground(windows_list::foreground());
        self.schedule_minute_tick();
        indexer::spawn_index(&self.paths);
        if self.cfg.borrow().autostart {
            familiar_sys::autostart::set_enabled(true);
        }
        self.lock.set_battery(self.battery.get());
        if self.cfg.borrow().lock_on_start {
            self.lock.show_with_splash(self);
        }
        log::info!("Familiar Core started");
    }

    fn start_services(&self) {
        let mut s = self.services.borrow_mut();
        let cfg = self.cfg.borrow();

        let hotkey = cfg.launcher.hotkey.trim().to_ascii_lowercase();
        let launcher_hotkey = if hotkey == "win" { None } else { Hotkey::parse(&hotkey) };
        if hotkey != "win" && launcher_hotkey.is_none() {
            log::warn!("invalid launcher hotkey {hotkey:?}");
        }
        let wallpaper_hotkey = self.cfg.borrow().wallpaper.alt_w;
        s.shell = ShellWatcher::spawn(ShellWatcherOptions { launcher_hotkey, wallpaper_hotkey }, |ev| {
            post(move |a| a.on_shell(ev))
        })
        .map_err(|e| log::error!("shell watcher: {e}"))
        .ok();
        s.privacy = PrivacyWatcher::spawn(|p| post(move |a| a.on_privacy(p)))
            .map_err(|e| log::warn!("privacy watcher: {e}"))
            .ok();
        if self.island.is_some() || self.widgets.as_ref().is_some_and(|w| w.wants_media()) {
            s.media = MediaService::spawn(|m| post(move |a| a.on_media(m)))
                .map_err(|e| log::warn!("media service: {e}"))
                .ok();
        }
        s.theme_watch = ThemeWatcher::spawn(&self.theme_path.borrow(), |t| post(move |a| a.set_theme(t)))
            .map_err(|e| log::warn!("theme hot reload disabled: {e}"))
            .ok();
        s.icons = IconWorker::spawn(&self.paths).map_err(|e| log::warn!("icon worker: {e}")).ok();
        drop(cfg);
        drop(s);
        self.sync_cursor_fx();
    }

    fn shutdown(&self) {
        log::info!("shutting down");
        self.audio.shutdown();
        // Dropping services joins their threads (hooks are removed first).
        *self.services.borrow_mut() = Services::default();
        if let Some(t) = &self.topbar {
            t.shutdown();
        }
        if let Some(d) = &self.dock {
            d.shutdown();
        }
        restore_everything();
    }

    pub fn quit(&self) {
        let _ = slint::quit_event_loop();
    }

    /// Starts a fresh copy (it waits for this one to exit) and quits.
    pub fn restart(&self) {
        let pid = std::process::id().to_string();
        match std::env::current_exe().and_then(|exe| std::process::Command::new(exe).args(["--after", &pid]).spawn()) {
            Ok(_) => self.quit(),
            Err(e) => log::error!("restart: {e}"),
        }
    }

    fn animations_on(&self) -> bool {
        self.cfg.borrow().window_animations && !self.calm.get()
    }

    pub fn save_config(&self) {
        if let Err(e) = self.cfg.borrow().save(&self.paths.config()) {
            log::warn!("saving config: {e}");
        }
    }

    pub fn theme_path(&self) -> PathBuf {
        self.theme_path.borrow().clone()
    }

    /// Switches to another theme file and hot-reloads from it from now on.
    pub fn switch_theme(&self, path: PathBuf, theme: ThemeSpec) {
        *self.theme_path.borrow_mut() = path;
        let watch = ThemeWatcher::spawn(&self.theme_path.borrow(), |t| post(move |a| a.set_theme(t)))
            .map_err(|e| log::warn!("theme hot reload disabled: {e}"))
            .ok();
        self.services.borrow_mut().theme_watch = watch;
        self.set_theme(theme);
    }

    /// Light/dark switch from the Control Center: Nothing Light or Dark.
    pub fn set_dark_mode(&self, dark: bool) {
        let stem = if dark { "nothing-dark" } else { "nothing-light" };
        let path = self.paths.resolve_theme(stem);
        match ThemeSpec::load(&path) {
            Ok(t) => {
                self.cfg.borrow_mut().theme = stem.into();
                self.save_config();
                self.switch_theme(path, t);
            }
            Err(e) => log::warn!("{}: {e}", path.display()),
        }
    }

    pub fn set_taskbar_hidden(&self, hide: bool) {
        if hide {
            self.hide_taskbar();
        } else if let Ok(mut guard) = RESTORE.lock()
            && let Some((state, marker)) = guard.take()
        {
            taskbar::restore(state);
            let _ = std::fs::remove_file(marker);
        }
    }

    // ----- taskbar --------------------------------------------------------

    fn hide_taskbar(&self) {
        let marker = self.paths.taskbar_marker();
        let mut guard = RESTORE.lock().unwrap_or_else(|e| e.into_inner());
        // Keep the original state if we already hid it once (Explorer restart).
        let state = match *guard {
            Some((state, _)) => {
                taskbar::hide();
                state
            }
            None => taskbar::hide(),
        };
        let _ = std::fs::write(&marker, if state.was_autohide { "autohide=1" } else { "autohide=0" });
        *guard = Some((state, marker));
    }

    // ----- theme ----------------------------------------------------------

    fn push_theme_everywhere(&self) {
        let t = self.theme.borrow();
        if let Some(x) = &self.topbar {
            ui::push_theme(&x.ui, &t);
        }
        if let Some(x) = &self.dock {
            ui::push_theme(&x.ui, &t);
        }
        if let Some(x) = &self.island {
            ui::push_theme(&x.ui, &t);
        }
        if let Some(w) = &self.widgets {
            for x in w.items.borrow().iter() {
                ui::push_theme(&x.ui, &t);
            }
        }
        ui::push_theme(&self.launcher.ui, &t);
        ui::push_theme(&self.settings.ui, &t);
    }

    pub fn set_theme(&self, theme: ThemeSpec) {
        log::info!("theme reloaded: {}", theme.meta.name);
        *self.theme.borrow_mut() = theme;
        if let Some(x) = &self.topbar {
            x.apply_theme(self);
        }
        if let Some(x) = &self.dock {
            x.apply_theme(self);
        }
        if let Some(x) = &self.island {
            x.apply_theme(self);
        }
        if let Some(x) = &self.widgets {
            x.apply_theme(self);
        }
        self.launcher.apply_theme(self);
        self.settings.theme_changed(self);
        if let Some(fx) = self.fx.borrow().as_ref() {
            fx.apply_theme(self);
        }
        self.sync_cursor_fx();
    }

    /// Installs or removes the low-level input hooks. They are removed in
    /// calm mode so a full-screen game pays nothing for them, not even the
    /// per-event hook call.
    fn sync_hooks(&self) {
        let calm = self.calm.get();
        let fx = self.theme.borrow().cursor.clone();
        let win_tap = self.cfg.borrow().launcher.hotkey.trim().eq_ignore_ascii_case("win");
        let mut s = self.services.borrow_mut();

        let want_keys = win_tap && !calm;
        if want_keys != s.win_key.is_some() {
            s.win_key = None;
            if want_keys {
                s.win_key = WinKeyHook::spawn(|| post(|a| a.launcher.toggle(a)))
                    .map_err(|e| log::warn!("Win key hook: {e}"))
                    .ok();
            }
        }

        // Respawned on every call: the theme may have switched which effects run.
        s.mouse = None;
        if (fx.ripple_on_click || fx.shake_to_find) && !calm {
            s.mouse = MouseHook::spawn(fx.ripple_on_click, fx.shake_to_find, |e| post(move |a| a.play_fx(e)))
                .map_err(|e| log::warn!("mouse hook: {e}"))
                .ok();
        }
    }

    /// Applies the theme's cursor scheme and effect hooks.
    fn sync_cursor_fx(&self) {
        self.sync_hooks();
        let fx = self.theme.borrow().cursor.clone();
        if fx.custom_scheme {
            let base = self.theme_path.borrow().parent().map(Path::to_path_buf).unwrap_or_default();
            let dir = base.join(fx.scheme_dir.as_deref().unwrap_or("cursors"));
            let n = cursor::apply_scheme(&dir);
            log::info!("applied {n} cursors from {}", dir.display());
            self.custom_cursors.set(n > 0);
        } else if self.custom_cursors.replace(false) {
            cursor::restore_system();
        }
    }

    fn play_fx(&self, e: familiar_sys::mouse::MouseFx) {
        if self.calm.get() {
            return;
        }
        if self.fx.borrow().is_none() {
            match CursorFx::new() {
                Ok(fx) => {
                    ui::push_theme(&fx.ui, &self.theme.borrow());
                    *self.fx.borrow_mut() = Some(fx);
                }
                Err(err) => {
                    log::warn!("cursor fx window: {err}");
                    return;
                }
            }
        }
        if let Some(fx) = self.fx.borrow().as_ref() {
            fx.play(self, e);
        }
    }

    // ----- events ---------------------------------------------------------

    fn on_shell(&self, ev: ShellEvent) {
        match ev {
            ShellEvent::Foreground(h) => self.on_foreground(h),
            ShellEvent::WindowsChanged => self.windows_changed(),
            ShellEvent::TitleChanged(_) => {}
            ShellEvent::WindowShown(h) if self.animations_on() => self.window_fx.open(self, h),
            ShellEvent::MinimizeEnd(h) if self.animations_on() => self.window_fx.restore(self, h),
            ShellEvent::MinimizeStart(h) if self.animations_on() => self.window_fx.minimize(self, h),
            ShellEvent::WindowShown(_) | ShellEvent::MinimizeStart(_) | ShellEvent::MinimizeEnd(_) => {}
            ShellEvent::EmergencyExit => {
                log::warn!("Ctrl+Alt+Q: emergency exit");
                self.quit();
            }
            ShellEvent::LauncherHotkey => self.launcher.toggle(self),
            ShellEvent::WallpaperHotkey => self.wallpaper_fx.next(self),
            ShellEvent::ExplorerRestarted => {
                log::info!("Explorer restarted; re-registering");
                if self.cfg.borrow().hide_taskbar {
                    self.hide_taskbar();
                }
                if let Some(t) = &self.topbar {
                    t.reregister(self);
                }
                if let Some(d) = &self.dock {
                    d.reregister(self);
                }
                self.windows_changed();
            }
            ShellEvent::SessionEnding => {
                restore_everything();
                self.quit();
            }
            ShellEvent::DisplayChanged => self.on_display_changed(),
            ShellEvent::PowerChanged => {
                self.battery.set(power::battery());
                if let Some(t) = &self.topbar {
                    t.set_battery(self.battery.get());
                }
                self.update_audio_state();
            }
        }
    }

    fn on_foreground(&self, fg: Hwnd) {
        if self.cfg.borrow().hide_taskbar && RESTORE.lock().is_ok_and(|g| g.is_some()) {
            // Explorer re-shows its taskbar on some Start/notification paths.
            taskbar::hide_windows();
        }
        if let Some(t) = &self.topbar {
            t.set_app_name(&self.app_name_for(fg));
            t.set_layout_code(&power::keyboard_layout_code());
        }
        if let Some(d) = &self.dock {
            d.set_foreground(fg);
        }
        if let Some(w) = &self.widgets
            && !self.cfg.borrow().widgets.attach_to_wallpaper
        {
            let class = windows_list::class_name(fg.get());
            w.desktop_in_front(matches!(class.as_str(), "Progman" | "WorkerW"));
        }
        self.evaluate_calm();
    }

    fn windows_changed(&self) {
        // Window churn arrives in bursts (create + show + name change...).
        self.windows_debounce.start(TimerMode::SingleShot, Duration::from_millis(120), || {
            with_app(|a| {
                if let Some(d) = &a.dock {
                    d.rebuild(a);
                }
            })
        });
    }

    /// Monitor layout, resolution or DPI changed: re-place every surface.
    pub fn on_display_changed(&self) {
        let old = self.monitor.get();
        let new = primary_monitor();
        self.monitor.set(new);
        let geometry_changed = old.bounds != new.bounds || old.scale != new.scale;
        if geometry_changed {
            // Only real monitor changes re-reserve AppBars; reserving itself
            // changes the work area and would otherwise loop.
            if let Some(t) = &self.topbar {
                t.relayout(self);
            }
            if let Some(d) = &self.dock {
                d.apply_theme(self);
            }
            if let Some(i) = &self.island {
                i.relayout(self);
            }
        }
        if (old.work != new.work || geometry_changed)
            && let Some(w) = &self.widgets
        {
            w.relayout(self);
        }
    }

    fn on_privacy(&self, p: PrivacyState) {
        if let Some(t) = &self.topbar {
            t.set_privacy(&p);
        }
        if let Some(i) = &self.island {
            i.set_privacy(&p);
        }
        *self.privacy.borrow_mut() = p;
    }

    fn on_media(&self, m: Option<NowPlaying>) {
        if let Some(i) = &self.island {
            i.set_media(self, m.as_ref());
        }
        self.control_center.set_media(m.as_ref());
        if let Some(w) = &self.widgets {
            w.set_media(m.as_ref());
        }
        self.lock.set_media(m.as_ref());
        *self.media.borrow_mut() = m;
        self.update_audio_state();
    }

    pub fn media(&self) -> Option<NowPlaying> {
        self.media.borrow().clone()
    }

    pub fn media_command(&self, cmd: MediaCommand) {
        if let Some(m) = self.services.borrow().media.as_ref() {
            m.send(cmd);
        }
    }

    /// Clicking the artwork brings the playing app to the front.
    pub fn focus_media_source(&self) {
        let Some(src) = self.media.borrow().as_ref().map(|m| m.source_app.clone()) else { return };
        let windows = windows_list::list_app_windows();
        let hit = windows.iter().find(|w| {
            w.aumid.as_deref().is_some_and(|a| a.eq_ignore_ascii_case(&src))
                || Path::new(&w.exe_path).file_name().is_some_and(|f| f.to_string_lossy().eq_ignore_ascii_case(&src))
        });
        match hit {
            Some(w) => windows_list::activate(w.hwnd),
            None if src.contains('!') => self.launch(&src),
            None => {}
        }
    }

    // ----- calm mode ------------------------------------------------------

    pub fn is_calm(&self) -> bool {
        self.calm.get()
    }

    pub fn on_fullscreen_hint(&self, _open: bool) {
        self.evaluate_calm();
    }

    fn evaluate_calm(&self) {
        let enabled = self.cfg.borrow().calm.pause_on_fullscreen;
        let calm = enabled && windows_list::foreground_is_fullscreen();
        if self.calm.replace(calm) != calm {
            log::info!("calm mode {}", if calm { "on (full-screen app)" } else { "off" });
            self.launcher.hide();
            self.sync_hooks();
            if let Some(t) = &self.topbar {
                t.set_visible(!calm);
            }
            if let Some(d) = &self.dock {
                d.set_visible(!calm);
            }
            if let Some(i) = &self.island {
                i.set_visible(!calm);
            }
            if let Some(w) = &self.widgets {
                w.set_visible(!calm);
            }
        }
        self.update_audio_state();
    }

    fn update_audio_state(&self) {
        let playing = self.media.borrow().as_ref().is_some_and(|m| m.playing);
        let saver = self.cfg.borrow().calm.pause_on_battery_saver && self.battery.get().saver;
        self.audio.set_active(playing && !self.calm.get() && !saver);
    }

    pub fn render_audio(&self, bands: &[f32]) {
        if let Some(i) = &self.island {
            let mut m = familiar_signal::Matrix::new(14, 7);
            m.draw_bars(bands);
            let style = familiar_signal::DotStyle {
                pitch: 4,
                radius: 0.4,
                on: familiar_signal::Rgba(255, 255, 255, 255),
                off: familiar_signal::Rgba(255, 255, 255, 28),
            };
            let (w, h, px) = familiar_signal::raster::render(&m, &style);
            i.set_spectrum(ui::image_from_rgba(w, h, &px));
        }
        if let Some(w) = &self.widgets {
            w.on_audio(self, bands);
        }
    }

    // ----- clock ----------------------------------------------------------

    fn schedule_minute_tick(&self) {
        let delay = Duration::from_millis(clock::ms_to_next_minute() + 20);
        self.minute_align.start(TimerMode::SingleShot, delay, || {
            with_app(|a| {
                a.minute_tick();
                a.minute.start(TimerMode::Repeated, Duration::from_secs(60), || with_app(|a| a.minute_tick()));
            })
        });
    }

    fn minute_tick(&self) {
        if let Some(t) = &self.topbar {
            t.refresh_clock(self);
            self.battery.set(power::battery());
            t.set_battery(self.battery.get());
            t.set_layout_code(&power::keyboard_layout_code());
        }
        if let Some(w) = &self.widgets {
            w.tick_clock(self);
        }
        self.lock.refresh_clock(self);
        self.lock.set_battery(self.battery.get());
    }

    // ----- apps -----------------------------------------------------------

    pub fn launch(&self, target: &str) {
        if target.is_empty() {
            return;
        }
        match apps::launch(target) {
            Ok(()) => {
                if let Some(d) = &self.dock {
                    d.bounce(self, target);
                }
                let _ = self.store.borrow().record_launch(target);
            }
            Err(e) => log::warn!("launching {target}: {e}"),
        }
    }

    /// Display name and icon path for a running app that may not be pinned.
    pub fn describe_app(&self, key: &str, w: &AppWindow) -> (String, PathBuf) {
        let stem = Path::new(&w.exe_path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let found = self
            .launcher
            .find_by_target(key)
            .or_else(|| if w.exe_path.is_empty() { None } else { self.launcher.find_by_exe(&w.exe_path) })
            // Store apps run from WindowsApps; their index entry is keyed by
            // package family ("Claude_pzs8sxrjxfjjc!Claude").
            .or_else(|| package_family(&w.exe_path).and_then(|f| self.launcher.find_by_family(&f)))
            .or_else(|| if stem.is_empty() { None } else { self.launcher.find_by_name(&stem) });
        if let Some(a) = found {
            if !a.icon_cache_path.is_empty() {
                return (a.name, PathBuf::from(a.icon_cache_path));
            }
            return (a.name, self.icon_path_for(key));
        }
        let name = match stem.to_ascii_lowercase().as_str() {
            "explorer" => "Explorador de archivos".to_string(),
            "" => w.title.clone(),
            // "notepad" → "Notepad": nicer than the raw file name.
            _ => {
                let mut c = stem.chars();
                c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
            }
        };
        (name, self.icon_path_for(key))
    }

    fn icon_path_for(&self, key: &str) -> PathBuf {
        indexer::icon_path(&self.paths.icon_dir(), key)
    }

    pub fn request_icon(&self, key: &str, exe: &str) {
        if key.is_empty() || !self.requested_icons.borrow_mut().insert(key.to_lowercase()) {
            return;
        }
        if let Some(w) = self.services.borrow().icons.as_ref() {
            w.request(key, exe);
        }
    }

    pub fn on_icon_ready(&self, key: &str, path: &Path) {
        let _ = self.store.borrow().set_dock_icon(key, &path.to_string_lossy());
        if let Some(d) = &self.dock {
            d.icon_ready(key, path);
        }
    }

    fn app_name_for(&self, fg: Hwnd) -> String {
        let class = windows_list::class_name(fg.get());
        if fg.is_null() || matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd") {
            return "Escritorio".into();
        }
        let pid = windows_list::process_id(fg.get());
        if pid == std::process::id() {
            return "Familiar".into();
        }
        if class == "CabinetWClass" {
            return "Explorador de archivos".into();
        }
        let w = AppWindow {
            hwnd: fg,
            title: windows_list::title(fg.get()),
            exe_path: windows_list::process_path(pid),
            aumid: windows_list::window_aumid(fg.get()),
            minimized: false,
        };
        self.describe_app(&w.app_key(), &w).0
    }

    pub fn on_index_ready(&self, entries: Vec<AppEntry>) {
        {
            let mut store = self.store.borrow_mut();
            if let Err(e) = store.replace_apps(&entries) {
                log::error!("saving app index: {e}");
            }
            if let Ok(apps) = store.apps() {
                self.launcher.set_apps(apps);
            }
        }
        self.seed_dock();
        if let Some(d) = &self.dock {
            d.rebuild(self);
        }
    }

    /// First run: pin a few well-known apps so the dock is not empty.
    fn seed_dock(&self) {
        let marker = self.paths.seeded_marker();
        if marker.exists() || self.launcher.app_count() == 0 {
            return;
        }
        let store = self.store.borrow();
        if store.dock_items().map(|d| !d.is_empty()).unwrap_or(true) {
            let _ = std::fs::write(&marker, "");
            return;
        }
        const DEFAULTS: &[&[&str]] = &[
            &["File Explorer", "Explorador de archivos"],
            &["Microsoft Edge"],
            &["Google Chrome"],
            &["Firefox", "Mozilla Firefox"],
            &["Spotify"],
            &["Visual Studio Code"],
            &["Terminal", "Windows Terminal"],
            &["Settings", "Configuración"],
        ];
        for names in DEFAULTS {
            if let Some(a) = names.iter().find_map(|n| self.launcher.find_by_name(n)) {
                let _ = store.pin(&NewDockItem {
                    app_id: &a.launch_target,
                    display_name: &a.name,
                    executable_path: &a.executable_path,
                    icon_cache_path: &a.icon_cache_path,
                });
            }
        }
        let _ = std::fs::write(&marker, "");
    }
}

fn print_usage() {
    println!(
        "familiar-core [--restore-taskbar] [--export-theme] [--import-theme <FAM1:...>] [--list-themes]\n\
         \n  --restore-taskbar   show the Windows taskbar again (after a crash) and exit\
         \n  --export-theme      print the active theme as a share code\
         \n  --import-theme CODE save a shared theme and make it active\
         \n  --list-themes       list installed themes\
         \n\nEmergency exit while running: Ctrl+Alt+Q"
    );
}

fn restore_from_marker(paths: &Paths) -> bool {
    let marker = paths.taskbar_marker();
    let Ok(content) = std::fs::read_to_string(&marker) else { return false };
    taskbar::restore(TaskbarState { was_autohide: content.trim() == "autohide=1" });
    let _ = std::fs::remove_file(marker);
    true
}

fn slug(name: &str) -> String {
    let s: String =
        name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() { "shared-theme".into() } else { s }
}

/// Handles one-shot CLI commands. Returns `Some(exit_code)` when the
/// process should exit instead of starting the shell.
fn run_cli(args: &[String], paths: &Paths) -> Option<i32> {
    let cmd = args.get(1).map(String::as_str)?;
    let cfg_path = paths.config();
    match cmd {
        "--help" | "-h" => {
            print_usage();
            Some(0)
        }
        "--restore-taskbar" => {
            if !restore_from_marker(paths) {
                taskbar::restore(TaskbarState { was_autohide: false });
            }
            println!("taskbar restored");
            Some(0)
        }
        "--export-theme" => {
            let cfg = AppConfig::load_or_create(&cfg_path);
            let theme = ThemeSpec::load(&paths.resolve_theme(&cfg.theme)).unwrap_or_default();
            match theme.to_share_code() {
                Ok(code) => {
                    println!("{code}");
                    Some(0)
                }
                Err(e) => {
                    eprintln!("{e}");
                    Some(1)
                }
            }
        }
        "--import-theme" => {
            let Some(code) = args.get(2) else {
                eprintln!("usage: --import-theme FAM1:...");
                return Some(2);
            };
            let theme = match ThemeSpec::from_share_code(code) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("{e}");
                    return Some(1);
                }
            };
            let stem = slug(&theme.meta.name);
            let path = paths.themes_dir().join(format!("{stem}.toml"));
            let written = theme
                .to_toml()
                .map_err(|e| e.to_string())
                .and_then(|s| std::fs::write(&path, s).map_err(|e| e.to_string()));
            if let Err(e) = written {
                eprintln!("{e}");
                return Some(1);
            }
            let mut cfg = AppConfig::load_or_create(&cfg_path);
            cfg.theme = stem.clone();
            let _ = cfg.save(&cfg_path);
            println!("installed theme {:?} as {stem}", theme.meta.name);
            Some(0)
        }
        "--list-themes" => {
            if let Ok(dir) = std::fs::read_dir(paths.themes_dir()) {
                for e in dir.flatten() {
                    let p = e.path();
                    if p.extension().is_some_and(|x| x == "toml") {
                        let name = ThemeSpec::load(&p).map(|t| t.meta.name).unwrap_or_else(|e| format!("<{e}>"));
                        println!("{:<20} {name}", p.file_stem().unwrap_or_default().to_string_lossy());
                    }
                }
            }
            Some(0)
        }
        other => {
            eprintln!("unknown argument {other:?}");
            print_usage();
            Some(2)
        }
    }
}

/// `FAMILIAR_FONT_FILE=C:\path\font.ttf` registers an extra font, for
/// systems where the Segoe families are missing (with `FAMILIAR_FONT` set
/// to its family name).
fn register_extra_font() {
    let Some(path) = std::env::var_os("FAMILIAR_FONT_FILE") else { return };
    match std::fs::read(&path) {
        Ok(bytes) => {
            let mut fonts = slint::fontique_011::shared_collection();
            let families = fonts.register_fonts(bytes.into(), None);
            log::info!("registered {} font families from {}", families.len(), Path::new(&path).display());
        }
        Err(e) => log::warn!("FAMILIAR_FONT_FILE: {e}"),
    }
}

#[cfg(not(feature = "winit"))]
fn select_backend() -> Result<(), slint::PlatformError> {
    let platform = crate::platform::LayeredPlatform::new()?;
    slint::platform::set_platform(Box::new(platform)).map_err(|e| slint::PlatformError::Other(e.to_string()))
}

#[cfg(feature = "winit")]
fn select_backend() -> Result<(), slint::PlatformError> {
    use slint::winit_030::winit::platform::windows::WindowAttributesExtWindows;
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .with_winit_window_attributes_hook(|attrs| {
            // Only the launcher takes focus when shown; every other surface
            // must leave the user's active window alone.
            let takes_focus = attrs.title == "Familiar Launcher";
            attrs.with_skip_taskbar(true).with_active(takes_focus).with_undecorated_shadow(false)
        })
        .select()
}

pub fn main() -> i32 {
    // Per-monitor DPI awareness, before any window exists. Without it Windows
    // bitmap-stretches every surface (blurry at 125 %) and hands out scaled
    // monitor rects, so the AppBar reservations land in the wrong place.
    unsafe {
        use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let args: Vec<String> = std::env::args().collect();
    let paths = Paths::resolve();
    if let Err(e) = paths.ensure() {
        eprintln!("cannot create {}: {e}", paths.data.display());
        return 1;
    }
    logger::init(&paths.log());
    if args.get(1).is_none_or(|a| a != "--after")
        && let Some(code) = run_cli(&args, &paths)
    {
        return code;
    }

    // `--after <pid>`: restarted from the settings; wait for the old copy.
    let restarted = args.get(1).is_some_and(|a| a == "--after");
    let mut instance = None;
    for _ in 0..if restarted { 50 } else { 1 } {
        instance = familiar_sys::util::SingleInstance::acquire("Local\\FamiliarCore.SingleInstance");
        if instance.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    let Some(_instance) = instance else {
        log::info!("already running");
        return 0;
    };
    // Whatever happens from here on, give the taskbar back.
    familiar_sys::safety::install(restore_everything);
    if restore_from_marker(&paths) {
        log::warn!("previous session did not exit cleanly; taskbar restored");
    }

    let cfg = AppConfig::load_or_create(&paths.config());
    if let Err(e) = paths.install_builtin_themes() {
        log::warn!("installing built-in themes: {e}");
    }
    let theme_path = paths.resolve_theme(&cfg.theme);
    let theme = ThemeSpec::load(&theme_path).unwrap_or_else(|e| {
        log::warn!("{}: {e}; using Ember Dark", theme_path.display());
        ThemeSpec::default()
    });
    let store = match Store::open(&paths.db()) {
        Ok(s) => s,
        Err(e) => {
            log::error!("opening {}: {e}", paths.db().display());
            return 1;
        }
    };

    if let Err(e) = select_backend() {
        log::error!("UI backend: {e}");
        return 1;
    }
    register_extra_font();
    let app = match App::new(cfg, paths, store, theme, theme_path) {
        Ok(a) => Rc::new(a),
        Err(e) => {
            log::error!("creating windows: {e}");
            return 1;
        }
    };
    APP.with(|a| *a.borrow_mut() = Some(app.clone()));
    app.start();

    let result = slint::run_event_loop_until_quit();
    app.shutdown();
    APP.with(|a| a.borrow_mut().take());
    if let Err(e) = result {
        log::error!("event loop: {e}");
        return 1;
    }
    0
}

/// Package family name ("Claude_pzs8sxrjxfjjc") of an exe under
/// `WindowsApps\Claude_4.29.0.0_x64__pzs8sxrjxfjjc\...`.
fn package_family(exe: &str) -> Option<String> {
    let lower = exe.to_ascii_lowercase();
    const DIR: &str = "\\windowsapps\\";
    let at = lower.find(DIR)? + DIR.len();
    let folder = exe[at..].split('\\').next()?;
    let (head, publisher) = folder.rsplit_once("__")?;
    let name = head.split('_').next()?;
    Some(format!("{name}_{publisher}"))
}

#[cfg(test)]
mod tests {
    use super::slug;

    #[test]
    fn package_families() {
        assert_eq!(
            super::package_family(r"C:\Program Files\WindowsApps\Claude_4.29.0.0_x64__pzs8sxrjxfjjc\app\Claude.exe")
                .as_deref(),
            Some("Claude_pzs8sxrjxfjjc")
        );
        assert_eq!(super::package_family(r"C:\Windows\notepad.exe"), None);
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("Ember Dark"), "ember-dark");
        assert_eq!(slug("¡¡"), "shared-theme");
    }
}
