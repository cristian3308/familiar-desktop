//! W2: the floating dock with cosine magnification.
//!
//! Pinned apps come from SQLite; running windows (from the shell watcher)
//! are matched to them by AppUserModelID or executable path. Unpinned
//! running apps follow a separator.
//!
//! At rest the native window is only as tall as the plate (that strip is
//! reserved as an AppBar), so it never covers other windows. While hovered
//! it grows upward to make room for magnified icons and the tooltip.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use familiar_layout::dock::{DockMetrics, DockSlot, SlotKind, hit_test, layout_dock};
use familiar_layout::easing::approach;
use familiar_store::NewDockItem;
use familiar_sys::Hwnd;
use familiar_sys::appbar::{AppBar, Edge};
use familiar_sys::window::{self, Rect, SurfaceOptions};
use familiar_sys::windows_list::{self, AppWindow};
use slint::{ComponentHandle, Model, Timer, TimerMode, VecModel};

use crate::app::{App, with_app};
use crate::ui::{self, DockIcon, DockWindow};

const SPACING: f32 = 6.0;
const PADDING: f32 = 8.0;
const BOTTOM_MARGIN: f32 = 6.0;
const TOOLTIP_ROOM: f32 = 36.0;
const SIDE_ROOM: f32 = 24.0;

#[derive(Clone, Debug)]
struct Entry {
    app_id: String,
    name: String,
    exe_path: String,
    icon_path: PathBuf,
    pinned: bool,
    separator: bool,
    windows: Vec<Hwnd>,
}

pub struct Dock {
    pub ui: DockWindow,
    model: Rc<VecModel<DockIcon>>,
    entries: RefCell<Vec<Entry>>,
    slots: RefCell<Vec<DockSlot>>,
    hwnd: Cell<Option<Hwnd>>,
    appbar: RefCell<Option<AppBar>>,
    pointer: Cell<Option<f32>>,
    strength: Cell<f32>,
    target: Cell<f32>,
    grown: Cell<bool>,
    anim: Timer,
    /// Window width in logical px; fixed per entry set so hovering never
    /// resizes horizontally.
    width: Cell<f32>,
}

impl Dock {
    pub fn new() -> Result<Self, slint::PlatformError> {
        let ui = DockWindow::new()?;
        let model = Rc::new(VecModel::default());
        ui.set_icons(model.clone().into());
        ui.on_pointer_moved(|x| with_app(|a| a.dock.as_ref().map_or((), |d| d.on_pointer(a, Some(x)))));
        ui.on_hover_changed(|h| with_app(|a| a.dock.as_ref().map_or((), |d| d.on_hover(a, h))));
        ui.on_clicked(|x| with_app(|a| a.dock.as_ref().map_or((), |d| d.on_click(a, x, Click::Primary))));
        ui.on_right_clicked(|x| with_app(|a| a.dock.as_ref().map_or((), |d| d.on_click(a, x, Click::Secondary))));
        ui.on_middle_clicked(|x| with_app(|a| a.dock.as_ref().map_or((), |d| d.on_click(a, x, Click::Middle))));
        Ok(Self {
            ui,
            model,
            entries: RefCell::new(Vec::new()),
            slots: RefCell::new(Vec::new()),
            hwnd: Cell::new(None),
            appbar: RefCell::new(None),
            pointer: Cell::new(None),
            strength: Cell::new(0.0),
            target: Cell::new(0.0),
            grown: Cell::new(false),
            anim: Timer::default(),
            width: Cell::new(400.0),
        })
    }

    fn metrics(app: &App) -> DockMetrics {
        let t = app.theme.borrow();
        DockMetrics {
            icon_size: t.metrics.dock_icon_size as f32,
            magnification: t.metrics.dock_magnification,
            spacing: SPACING,
            ..DockMetrics::default()
        }
    }

    fn kinds(&self) -> Vec<SlotKind> {
        self.entries.borrow().iter().map(|e| if e.separator { SlotKind::Separator } else { SlotKind::Icon }).collect()
    }

    fn plate_height(m: &DockMetrics) -> f32 {
        m.icon_size + 2.0 * PADDING
    }

    fn resting_height(m: &DockMetrics) -> f32 {
        Self::plate_height(m) + BOTTOM_MARGIN
    }

    fn grown_height(m: &DockMetrics) -> f32 {
        Self::resting_height(m) + (m.max_icon_size() - m.icon_size) + TOOLTIP_ROOM
    }

    fn window_rect(&self, app: &App, grown: bool) -> Rect {
        let mon = app.monitor.get();
        let m = Self::metrics(app);
        let h = if grown { Self::grown_height(&m) } else { Self::resting_height(&m) };
        let (w, h) = ((self.width.get() * mon.scale).round() as i32, (h * mon.scale).round() as i32);
        Rect { x: mon.bounds.x + (mon.bounds.w - w) / 2, y: mon.bounds.y + mon.bounds.h - h, w, h }
    }

    pub fn show(&self, app: &App) {
        self.rebuild(app);
        let r = self.window_rect(app, false);
        let w = self.ui.window();
        w.set_position(slint::PhysicalPosition::new(r.x, r.y));
        w.set_size(slint::PhysicalSize::new(r.w as u32, r.h as u32));
        if let Err(e) = self.ui.show() {
            log::error!("dock: {e}");
            return;
        }
        let Some(hwnd) = ui::hwnd_of(w) else { return };
        self.hwnd.set(Some(hwnd));
        window::apply_surface(
            hwnd.get(),
            SurfaceOptions { tool_window: true, no_activate: true, topmost: true, click_through: false },
        );
        // The window is clipped to the plate (see `apply_region`), so the
        // backdrop takes the plate's shape.
        ui::apply_material(hwnd, &app.theme.borrow(), false);
        self.register_appbar(app);
        self.layout_icons(app);
    }

    fn register_appbar(&self, app: &App) {
        self.appbar.borrow_mut().take();
        if !app.cfg.borrow().dock.reserve_space {
            return;
        }
        let Some(hwnd) = self.hwnd.get() else { return };
        let mon = app.monitor.get();
        let thickness = (Self::resting_height(&Self::metrics(app)) * mon.scale).round() as i32;
        if let Some((bar, _)) = AppBar::register(hwnd.get(), Edge::Bottom, thickness, mon.bounds, false, |_| {}) {
            *self.appbar.borrow_mut() = Some(bar);
        }
    }

    pub fn reregister(&self, app: &App) {
        self.register_appbar(app);
    }

    pub fn shutdown(&self) {
        if let Some(bar) = self.appbar.borrow_mut().take() {
            bar.unregister();
        }
    }

    pub fn apply_theme(&self, app: &App) {
        ui::push_theme(&self.ui, &app.theme.borrow());
        if let Some(h) = self.hwnd.get() {
            ui::apply_material(h, &app.theme.borrow(), false);
        }
        if let Some(bar) = self.appbar.borrow().as_ref() {
            let mon = app.monitor.get();
            bar.set_monitor(mon.bounds);
            bar.set_thickness((Self::resting_height(&Self::metrics(app)) * mon.scale).round() as i32);
        }
        self.relayout(app);
    }

    pub fn set_visible(&self, visible: bool) {
        ui::set_native_visible(self.hwnd.get(), visible);
    }

    /// Recomputes window size and icon geometry (theme, monitor or entry change).
    pub fn relayout(&self, app: &App) {
        let m = Self::metrics(app);
        let kinds = self.kinds();
        self.width.set(m.max_width(&kinds) + 2.0 * PADDING + 2.0 * SIDE_ROOM);
        self.ui.set_plate_height(Self::plate_height(&m));
        self.ui.set_bottom_margin(BOTTOM_MARGIN);
        self.ui.set_icon_padding(PADDING);
        if let Some(hwnd) = self.hwnd.get() {
            window::set_bounds(hwnd.get(), self.window_rect(app, self.grown.get()));
        }
        self.layout_icons(app);
    }

    fn layout_icons(&self, app: &App) {
        let m = Self::metrics(app);
        let kinds = self.kinds();
        let (slots, total) = layout_dock(&kinds, &m, self.width.get(), self.pointer.get(), self.strength.get());
        let plate_w = total + 2.0 * PADDING;
        self.ui.set_plate_x((self.width.get() - plate_w) / 2.0);
        self.ui.set_plate_width(plate_w);
        for (i, s) in slots.iter().enumerate() {
            if let Some(mut row) = self.model.row_data(i)
                && (row.x != s.x || row.size != s.size)
            {
                row.x = s.x;
                row.size = s.size;
                self.model.set_row_data(i, row);
            }
        }
        let hovered = match self.pointer.get() {
            Some(x) if self.target.get() > 0.5 && self.strength.get() > 0.5 => {
                hit_test(&slots, x).filter(|i| kinds[*i] == SlotKind::Icon)
            }
            _ => None,
        };
        self.ui.set_hovered(hovered.map_or(-1, |i| i as i32));
        self.apply_region(app, &slots, plate_w, hovered);
        *self.slots.borrow_mut() = slots;
    }

    /// Clips the window to plate + magnified icons + tooltip. This shapes the
    /// DWM backdrop and lets clicks outside the dock reach the windows below.
    fn apply_region(&self, app: &App, slots: &[DockSlot], plate_w: f32, hovered: Option<usize>) {
        let Some(hwnd) = self.hwnd.get() else { return };
        let m = Self::metrics(app);
        let scale = app.monitor.get().scale;
        let px = |v: f32| (v * scale).round() as i32;
        let win_h = self.window_rect(app, self.grown.get()).h as f32 / scale;
        let plate_h = Self::plate_height(&m);
        let plate_y = win_h - BOTTOM_MARGIN - plate_h;
        let plate_x = (self.width.get() - plate_w) / 2.0;
        let icons_bottom = plate_y + plate_h - PADDING;
        let radius = app.theme.borrow().metrics.corner_radius as f32;

        let mut parts = vec![window::RegionPart {
            rect: Rect { x: px(plate_x), y: px(plate_y), w: px(plate_w), h: px(plate_h) },
            radius: px(radius),
        }];
        for s in slots {
            let top = icons_bottom - s.size;
            if top < plate_y {
                parts.push(window::RegionPart {
                    rect: Rect { x: px(s.x), y: px(top), w: px(s.size), h: px(plate_y - top + radius) },
                    radius: px(s.size * 0.22),
                });
            }
        }
        if let Some(i) = hovered {
            let s = slots[i];
            let (tw, th) = (self.ui.get_tip_width(), self.ui.get_tip_height());
            let tx = (s.x + s.size / 2.0 - tw / 2.0).min(self.width.get() - tw - 2.0).max(2.0);
            let ty = (icons_bottom - s.size - th - 6.0).max(0.0);
            parts.push(window::RegionPart {
                rect: Rect { x: px(tx), y: px(ty), w: px(tw), h: px(th) },
                radius: px(7.0),
            });
        }
        window::set_region(hwnd.get(), &parts);
    }

    fn on_pointer(&self, app: &App, x: Option<f32>) {
        self.pointer.set(x);
        self.layout_icons(app);
    }

    fn on_hover(&self, app: &App, hovered: bool) {
        self.target.set(if hovered { 1.0 } else { 0.0 });
        if hovered
            && !self.grown.replace(true)
            && let Some(h) = self.hwnd.get()
        {
            window::set_bounds(h.get(), self.window_rect(app, true));
            self.layout_icons(app);
        }
        // On leave the last pointer position is kept so the icons shrink
        // smoothly instead of snapping back.
        // Animate magnification strength at ~60 fps only while it changes.
        self.anim.start(TimerMode::Repeated, Duration::from_millis(16), || {
            with_app(|a| {
                if let Some(d) = a.dock.as_ref() {
                    d.tick(a);
                }
            })
        });
    }

    fn tick(&self, app: &App) {
        let s = approach(self.strength.get(), self.target.get(), 0.14);
        self.strength.set(s);
        self.layout_icons(app);
        if s == self.target.get() {
            self.anim.stop();
            if s == 0.0 {
                self.pointer.set(None);
                if self.grown.replace(false)
                    && let Some(h) = self.hwnd.get()
                {
                    window::set_bounds(h.get(), self.window_rect(app, false));
                }
                self.layout_icons(app);
            }
        }
    }

    /// Re-reads pinned items and running windows.
    pub fn rebuild(&self, app: &App) {
        let show_running = app.cfg.borrow().dock.show_running;
        let pinned = app.store.borrow().dock_items().unwrap_or_default();
        let mut windows = windows_list::list_app_windows();
        let fg = windows_list::foreground();

        let mut entries: Vec<Entry> = pinned
            .into_iter()
            .map(|p| {
                let mut e = Entry {
                    app_id: p.app_id.clone(),
                    name: p.display_name.clone(),
                    exe_path: p.executable_path.clone(),
                    icon_path: PathBuf::from(&p.icon_cache_path),
                    pinned: true,
                    separator: false,
                    windows: Vec::new(),
                };
                windows.retain(|w| {
                    let hit = matches(&e, w);
                    if hit {
                        e.windows.push(w.hwnd);
                    }
                    !hit
                });
                e
            })
            .collect();

        if show_running && !windows.is_empty() {
            let mut running: Vec<Entry> = Vec::new();
            for w in windows {
                let key = w.app_key();
                if key.is_empty() {
                    continue;
                }
                if let Some(e) = running.iter_mut().find(|e| e.app_id == key) {
                    e.windows.push(w.hwnd);
                    continue;
                }
                let (name, icon_path) = app.describe_app(&key, &w);
                running.push(Entry {
                    app_id: key,
                    name,
                    exe_path: w.exe_path.clone(),
                    icon_path,
                    pinned: false,
                    separator: false,
                    windows: vec![w.hwnd],
                });
            }
            if !running.is_empty() && !entries.is_empty() {
                entries.push(Entry {
                    app_id: String::new(),
                    name: String::new(),
                    exe_path: String::new(),
                    icon_path: PathBuf::new(),
                    pinned: false,
                    separator: true,
                    windows: Vec::new(),
                });
            }
            entries.extend(running);
        }

        for e in entries.iter().filter(|e| !e.separator && !e.icon_path.exists()) {
            app.request_icon(&e.app_id, &e.exe_path);
        }

        let rows: Vec<DockIcon> = entries
            .iter()
            .map(|e| DockIcon {
                icon: ui::load_icon(&e.icon_path),
                name: e.name.as_str().into(),
                running: !e.windows.is_empty(),
                active: e.windows.contains(&fg),
                separator: e.separator,
                x: 0.0,
                size: 0.0,
            })
            .collect();
        let structure_changed = {
            let old = self.entries.borrow();
            old.len() != entries.len() || old.iter().zip(&entries).any(|(a, b)| a.app_id != b.app_id)
        };
        *self.entries.borrow_mut() = entries;
        self.model.set_vec(rows);
        if structure_changed {
            self.relayout(app);
        } else {
            self.layout_icons(app);
        }
    }

    /// Cheap update when only the foreground window changed.
    pub fn set_foreground(&self, fg: Hwnd) {
        let entries = self.entries.borrow();
        for (i, e) in entries.iter().enumerate() {
            if let Some(mut row) = self.model.row_data(i) {
                let active = e.windows.contains(&fg);
                if row.active != active {
                    row.active = active;
                    self.model.set_row_data(i, row);
                }
            }
        }
    }

    /// An icon finished extracting in the background.
    pub fn icon_ready(&self, app_id: &str, path: &std::path::Path) {
        let mut entries = self.entries.borrow_mut();
        for (i, e) in entries.iter_mut().enumerate() {
            if e.app_id.eq_ignore_ascii_case(app_id) {
                e.icon_path = path.to_path_buf();
                if let Some(mut row) = self.model.row_data(i) {
                    row.icon = ui::load_icon(path);
                    self.model.set_row_data(i, row);
                }
            }
        }
    }

    fn on_click(&self, app: &App, x: f32, click: Click) {
        let Some(index) = hit_test(&self.slots.borrow(), x) else { return };
        let Some(entry) = self.entries.borrow().get(index).cloned() else { return };
        if entry.separator {
            return;
        }
        match click {
            Click::Primary if !entry.windows.is_empty() => {
                if entry.windows.len() == 1 {
                    windows_list::activate_or_minimize(entry.windows[0]);
                } else {
                    // Cycle through the app's windows.
                    let fg = windows_list::foreground();
                    let next = entry.windows.iter().position(|h| *h == fg).map_or(0, |i| (i + 1) % entry.windows.len());
                    windows_list::activate(entry.windows[next]);
                }
            }
            Click::Primary | Click::Middle => app.launch(&entry.app_id),
            Click::Secondary => {
                let store = app.store.borrow();
                let result = if entry.pinned {
                    store.unpin(&entry.app_id).map(|_| ())
                } else {
                    store
                        .pin(&NewDockItem {
                            app_id: &entry.app_id,
                            display_name: &entry.name,
                            executable_path: &entry.exe_path,
                            icon_cache_path: &entry.icon_path.to_string_lossy(),
                        })
                        .map(|_| ())
                };
                drop(store);
                if let Err(e) = result {
                    log::warn!("dock pin toggle failed: {e}");
                }
                self.rebuild(app);
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Click {
    Primary,
    Secondary,
    Middle,
}

fn matches(e: &Entry, w: &AppWindow) -> bool {
    if let Some(aumid) = &w.aumid
        && aumid.eq_ignore_ascii_case(&e.app_id)
    {
        return true;
    }
    !w.exe_path.is_empty()
        && (w.exe_path.eq_ignore_ascii_case(&e.exe_path) || w.exe_path.eq_ignore_ascii_case(&e.app_id))
}
