//! W6: the settings window (theme, accent, dock, widgets, general).
//!
//! Looks are written into the active theme file, so they go through the
//! same hot-reload path as hand edits. Behaviour goes into `config.toml`;
//! what cannot change live (which widgets exist) asks for a restart.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use familiar_sys::window::{self, Rect, SurfaceOptions};
use familiar_sys::{Hwnd, windows_list};
use familiar_theme::Theme as ThemeSpec;
use slint::{ComponentHandle, VecModel};

use crate::app::{App, with_app};
use crate::ui::{self, SettingsWindow, ThemeCard};

const WIDTH: f32 = 760.0;
const HEIGHT: f32 = 640.0;

/// Widget ids in the order the switches show them.
const WIDGETS: [&str; 8] =
    ["calendar", "weather", "system", "signal_01", "clock_grid", "visualizer", "analog_clock", "now_playing"];

pub struct Settings {
    pub ui: SettingsWindow,
    hwnd: Cell<Option<Hwnd>>,
    visible: Cell<bool>,
    /// Theme files offered in the picker, in display order.
    themes: std::cell::RefCell<Vec<PathBuf>>,
    pos: Cell<(i32, i32)>,
}

impl Settings {
    pub fn new() -> Result<Self, slint::PlatformError> {
        let ui = SettingsWindow::new()?;
        ui.on_changed(|| with_app(|a| a.settings.apply(a)));
        ui.on_theme_chosen(|i| with_app(|a| a.settings.choose_theme(a, i)));
        ui.on_city_committed(|c| {
            with_app(|a| {
                a.cfg.borrow_mut().widgets.weather_city = c.trim().to_string();
                a.save_config();
                if let Some(w) = &a.widgets {
                    w.refresh_weather(a);
                }
            })
        });
        ui.on_files_style_next(|| {
            with_app(|a| {
                let next = crate::dock::FilesStyle::from_id(&a.cfg.borrow().dock.files_style).next();
                a.cfg.borrow_mut().dock.files_style = next.id().into();
                a.save_config();
                a.settings.ui.set_files_style(next.name().into());
                if let Some(d) = &a.dock {
                    d.set_files_style(next);
                }
            })
        });
        ui.on_signal_mode_next(|| {
            with_app(|a| {
                if let Some(w) = &a.widgets {
                    w.next_signal_mode(a);
                }
            })
        });
        ui.on_restart(|| with_app(|a| a.restart()));
        ui.on_quit_familiar(|| with_app(|a| a.quit()));
        ui.on_close_window(|| with_app(|a| a.settings.hide()));
        ui.on_drag_moved(|dx, dy| with_app(|a| a.settings.drag(a, dx, dy)));
        Ok(Self {
            ui,
            hwnd: Cell::new(None),
            visible: Cell::new(false),
            themes: Default::default(),
            pos: Cell::new((0, 0)),
        })
    }

    pub fn toggle(&self, app: &App) {
        if self.visible.get() { self.hide() } else { self.show(app) }
    }

    pub fn show(&self, app: &App) {
        log::info!("settings: open");
        self.load(app);
        let mon = app.monitor.get();
        let (w, h) = ((WIDTH * mon.scale) as i32, (HEIGHT * mon.scale) as i32);
        if !self.visible.get() {
            self.pos.set((mon.work.x + (mon.work.w - w) / 2, mon.work.y + (mon.work.h - h) / 2));
        }
        let (x, y) = self.pos.get();
        let win = self.ui.window();
        win.set_position(slint::PhysicalPosition::new(x, y));
        win.set_size(slint::PhysicalSize::new(w as u32, h as u32));
        if let Err(e) = self.ui.show() {
            log::error!("settings: {e}");
            return;
        }
        self.visible.set(true);
        if let Some(hwnd) = ui::hwnd_of(win) {
            if self.hwnd.replace(Some(hwnd)).is_none() {
                window::apply_surface(
                    hwnd.get(),
                    SurfaceOptions { tool_window: false, no_activate: false, topmost: false, click_through: false },
                );
                ui::round_region(hwnd, win, 14.0);
            }
            window::bring_to_front(hwnd.get());
            windows_list::activate(hwnd);
        }
    }

    pub fn hide(&self) {
        if self.visible.replace(false) {
            let _ = self.ui.hide();
        }
    }

    fn drag(&self, app: &App, dx: f32, dy: f32) {
        let scale = app.monitor.get().scale;
        let (x, y) = self.pos.get();
        let (x, y) = (x + (dx * scale) as i32, y + (dy * scale) as i32);
        self.pos.set((x, y));
        if let Some(h) = self.hwnd.get() {
            let size = self.ui.window().size();
            window::set_bounds(h.get(), Rect { x, y, w: size.width as i32, h: size.height as i32 });
        }
    }

    /// Fills every control from the current theme and config.
    fn load(&self, app: &App) {
        ui::push_theme(&self.ui, &app.theme.borrow());
        let dir = app.paths.themes_dir();
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .map(|rd| {
                rd.filter_map(Result::ok)
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|x| x == "toml"))
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        let cards: Vec<ThemeCard> = files
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let t = ThemeSpec::load(p).unwrap_or_default();
                let opaque = |c: familiar_theme::Color| slint::Color::from_rgb_u8(c.r, c.g, c.b);
                ThemeCard {
                    name: t.meta.name.as_str().into(),
                    code: format!("FD-{:02}", i + 1).into(),
                    bg: opaque(t.colors.background_tint),
                    accent: opaque(t.colors.accent),
                    ink: opaque(t.colors.text_primary),
                }
            })
            .collect();
        let current = app.theme_path();
        let index = files.iter().position(|p| *p == current).map_or(-1, |i| i as i32);
        self.ui.set_themes(Rc::new(VecModel::from(cards)).into());
        self.ui.set_theme_index(index);
        *self.themes.borrow_mut() = files;

        let t = app.theme.borrow();
        self.ui.set_accent(ui::color(t.colors.accent));
        self.ui.set_bar_opacity(t.colors.bar_tint.a as f32 / 255.0);
        self.ui.set_icon_size(t.metrics.dock_icon_size as f32);
        self.ui.set_magnification(t.metrics.dock_magnification);
        drop(t);

        let cfg = app.cfg.borrow();
        self.ui.set_custom_icons(cfg.dock.custom_icons);
        self.ui.set_window_previews(cfg.dock.window_previews);
        self.ui.set_files_style(crate::dock::FilesStyle::from_id(&cfg.dock.files_style).name().into());
        self.ui.set_show_running(cfg.dock.show_running);
        let has = |id: &str| cfg.widgets.enabled && cfg.widgets.items.iter().any(|i| i == id);
        self.ui.set_w_calendar(has(WIDGETS[0]));
        self.ui.set_w_weather(has(WIDGETS[1]));
        self.ui.set_w_system(has(WIDGETS[2]));
        self.ui.set_w_signal(has(WIDGETS[3]));
        self.ui.set_w_clock(has(WIDGETS[4]));
        self.ui.set_w_visualizer(has(WIDGETS[5]));
        self.ui.set_w_analog(has(WIDGETS[6]));
        self.ui.set_w_music(has(WIDGETS[7]));
        self.ui.set_city(cfg.widgets.weather_city.as_str().into());
        self.ui.set_signal_mode(crate::widgets::signal_mode(&cfg).name().into());
        self.ui.set_hide_taskbar(cfg.hide_taskbar);
        self.ui.set_autostart(cfg.autostart);
        self.ui.set_lock_on_start(cfg.lock_on_start);
        self.ui.set_window_animations(cfg.window_animations);
        self.ui.set_genie_effect(cfg.genie_effect);
    }

    pub fn refresh_signal_mode(&self, mode: familiar_signal::signal::SignalMode) {
        self.ui.set_signal_mode(mode.name().into());
    }

    /// Reads every control and applies what changed.
    fn apply(&self, app: &App) {
        // Looks → active theme file (hot reload picks the write up too).
        let mut t = app.theme.borrow().clone();
        let a = self.ui.get_accent();
        t.colors.accent = familiar_theme::Color { r: a.red(), g: a.green(), b: a.blue(), a: 255 };
        t.colors.bar_tint.a = (self.ui.get_bar_opacity().clamp(0.0, 1.0) * 255.0).round() as u8;
        t.metrics.dock_icon_size = self.ui.get_icon_size().round() as u32;
        t.metrics.dock_magnification = (self.ui.get_magnification() * 100.0).round() / 100.0;
        t.sanitize();
        if t != *app.theme.borrow() {
            match t.to_toml() {
                Ok(src) => {
                    if let Err(e) = std::fs::write(app.theme_path(), src) {
                        log::warn!("saving theme: {e}");
                    }
                }
                Err(e) => log::warn!("encoding theme: {e}"),
            }
            app.set_theme(t);
        }

        // Behaviour → config.toml.
        let flags = [
            self.ui.get_w_calendar(),
            self.ui.get_w_weather(),
            self.ui.get_w_system(),
            self.ui.get_w_signal(),
            self.ui.get_w_clock(),
            self.ui.get_w_visualizer(),
            self.ui.get_w_analog(),
            self.ui.get_w_music(),
        ];
        let items: Vec<String> =
            WIDGETS.iter().zip(flags).filter(|(_, on)| *on).map(|(id, _)| (*id).to_string()).collect();
        let (dock_changed, widgets_changed, taskbar, autostart) = {
            let mut cfg = app.cfg.borrow_mut();
            let dock_changed = cfg.dock.custom_icons != self.ui.get_custom_icons()
                || cfg.dock.show_running != self.ui.get_show_running();
            cfg.dock.custom_icons = self.ui.get_custom_icons();
            cfg.dock.window_previews = self.ui.get_window_previews();
            cfg.dock.show_running = self.ui.get_show_running();
            // Keep the user's order for widgets they already had.
            let mut new_items: Vec<String> = cfg.widgets.items.iter().filter(|i| items.contains(i)).cloned().collect();
            new_items.extend(items.iter().filter(|i| !cfg.widgets.items.contains(i)).cloned());
            let widgets_changed = new_items != cfg.widgets.items || cfg.widgets.enabled == new_items.is_empty();
            cfg.widgets.items = new_items;
            cfg.widgets.enabled = !cfg.widgets.items.is_empty();
            let taskbar = (cfg.hide_taskbar != self.ui.get_hide_taskbar()).then_some(self.ui.get_hide_taskbar());
            cfg.hide_taskbar = self.ui.get_hide_taskbar();
            let autostart = (cfg.autostart != self.ui.get_autostart()).then_some(self.ui.get_autostart());
            cfg.autostart = self.ui.get_autostart();
            cfg.lock_on_start = self.ui.get_lock_on_start();
            cfg.window_animations = self.ui.get_window_animations();
            cfg.genie_effect = self.ui.get_genie_effect();
            (dock_changed, widgets_changed, taskbar, autostart)
        };
        app.save_config();
        if dock_changed && let Some(d) = &app.dock {
            d.rebuild(app);
        }
        if let Some(hide) = taskbar {
            app.set_taskbar_hidden(hide);
        }
        if let Some(on) = autostart {
            familiar_sys::autostart::set_enabled(on);
        }
        if widgets_changed && let Some(w) = &app.widgets {
            let items = app.cfg.borrow().widgets.items.clone();
            w.sync(app, &items);
        }
    }

    fn choose_theme(&self, app: &App, index: i32) {
        let Some(path) = self.themes.borrow().get(index.max(0) as usize).cloned() else { return };
        match ThemeSpec::load(&path) {
            Ok(t) => {
                let stem = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
                app.cfg.borrow_mut().theme = stem;
                app.save_config();
                app.switch_theme(path, t);
                self.load(app);
            }
            Err(e) => log::warn!("{}: {e}", path.display()),
        }
    }

    /// Keeps the open window in step after a hot reload.
    pub fn theme_changed(&self, app: &App) {
        ui::push_theme(&self.ui, &app.theme.borrow());
        if self.visible.get() {
            self.ui.set_accent(ui::color(app.theme.borrow().colors.accent));
        }
    }
}
