//! W2: the floating dock with cosine magnification.
//!
//! Pinned apps come from SQLite; running windows (from the shell watcher)
//! are matched to them by AppUserModelID or executable path. Unpinned
//! running apps follow a separator.
//!
//! At rest the native window is only as tall as the plate (that strip is
//! reserved as an AppBar), so it never covers other windows. While hovered
//! it grows upward to make room for magnified icons and the tooltip.
//!
//! Resting on a running app lists its open windows above the icon; a click
//! on a title switches to that window.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

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
const PADDING: f32 = 6.0;
const BOTTOM_MARGIN: f32 = 8.0;
/// Layered windows let clicks through wherever a pixel is fully transparent,
/// so the dock window can stay at its grown size: no resize when the pointer
/// enters and no window region to recompute on every move, which is what
/// made magnification stutter. The `winit` build still needs both.
const LAYERED: bool = cfg!(not(feature = "winit"));
/// Launch target of the trash tile.
const TRASH: &str = "shell:RecycleBinFolder";
const TOOLTIP_ROOM: f32 = 36.0;
/// Space above the icons for the window list (transparent, click-through).
const PREVIEW_ROOM: f32 = 230.0;
/// Most windows listed for one app.
const PREVIEW_MAX: usize = 6;
const PREVIEW_DELAY: Duration = Duration::from_millis(420);
const PREVIEW_GRACE: Duration = Duration::from_millis(320);
const SIDE_ROOM: f32 = 24.0;
/// Seconds per launch-bounce hop.
const BOUNCE_PERIOD: f32 = 0.6;
/// Hops before giving up on an app that never shows a window.
const BOUNCE_MAX_HOPS: u32 = 6;

/// Look of the Files (Explorer) icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilesStyle {
    Classic,
    DotF,
    Mono,
    Theme,
    Glass,
}

impl FilesStyle {
    const ALL: [FilesStyle; 5] =
        [FilesStyle::Classic, FilesStyle::DotF, FilesStyle::Mono, FilesStyle::Theme, FilesStyle::Glass];

    pub fn id(self) -> &'static str {
        match self {
            FilesStyle::Classic => "classic",
            FilesStyle::DotF => "dot",
            FilesStyle::Mono => "mono",
            FilesStyle::Theme => "theme",
            FilesStyle::Glass => "glass",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            FilesStyle::Classic => "Classic",
            FilesStyle::DotF => "Dot F",
            FilesStyle::Mono => "Mono",
            FilesStyle::Theme => "Theme",
            FilesStyle::Glass => "Glass",
        }
    }

    /// Unknown ids fall back to the classic icon.
    pub fn from_id(id: &str) -> Self {
        Self::ALL.into_iter().find(|s| s.id().eq_ignore_ascii_case(id)).unwrap_or(FilesStyle::Classic)
    }

    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    /// `variant` of FilesIcon in dock.slint.
    fn ui_index(self) -> i32 {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0) as i32
    }
}

/// An icon bouncing while its app starts (macOS-style launch feedback).
struct Bounce {
    app_id: String,
    start: Instant,
    /// Stops after this many hops; lowered once the app has a window.
    hops: u32,
}

impl Bounce {
    fn elapsed_hops(&self) -> f32 {
        self.start.elapsed().as_secs_f32() / BOUNCE_PERIOD
    }

    fn done(&self) -> bool {
        self.elapsed_hops() >= self.hops as f32
    }

    /// Height above the plate in logical px: a series of decaying hops.
    fn lift(&self, icon_size: f32) -> f32 {
        let t = self.elapsed_hops();
        if t >= self.hops as f32 {
            return 0.0;
        }
        let amp = icon_size * 0.5 * (1.0 - 0.1 * t.floor()).max(0.4);
        amp * (std::f32::consts::PI * t.fract()).sin()
    }
}

/// What a dock tile is. Built-in tiles are drawn by the UI, not from icons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Special {
    App,
    Launcher,
    Trash,
}

impl Special {
    /// Matches `DockIcon.kind` in `dock.slint`.
    fn ui_kind(self) -> i32 {
        match self {
            Special::App => 0,
            Special::Launcher => 1,
            Special::Trash => 2,
        }
    }
}

/// Built-in macOS-style skin for some of Windows' own apps (`DockIcon.kind`
/// 3–5), or 0 to use the app's real icon.
fn system_skin(app_id: &str, exe: &str) -> i32 {
    let id = app_id.to_ascii_lowercase();
    let exe = exe.to_ascii_lowercase();
    let file = exe.rsplit(['\\', '/']).next().unwrap_or_default();
    if file == "explorer.exe" || id == "microsoft.windows.explorer" {
        3
    } else if id.contains("immersivecontrolpanel") || file == "systemsettings.exe" {
        4
    } else if id.contains("windowsterminal")
        || matches!(file, "windowsterminal.exe" | "cmd.exe" | "powershell.exe" | "pwsh.exe")
    {
        5
    } else {
        0
    }
}

#[derive(Clone, Debug)]
struct Entry {
    app_id: String,
    name: String,
    exe_path: String,
    icon_path: PathBuf,
    pinned: bool,
    separator: bool,
    special: Special,
    windows: Vec<Hwnd>,
}

impl Entry {
    fn builtin(special: Special, name: &str) -> Self {
        Self {
            app_id: String::new(),
            name: name.into(),
            exe_path: String::new(),
            icon_path: PathBuf::new(),
            pinned: false,
            separator: false,
            special,
            windows: Vec::new(),
        }
    }

    fn separator() -> Self {
        Self { separator: true, ..Self::builtin(Special::App, "") }
    }
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
    bounces: RefCell<Vec<Bounce>>,
    bounce_anim: Timer,
    /// Index of the icon being dragged to a new place.
    dragging: Cell<Option<usize>>,
    /// Window width in logical px; fixed per entry set so hovering never
    /// resizes horizontally.
    width: Cell<f32>,
    /// Entry whose window list is shown, and its windows in list order.
    preview: Cell<Option<usize>>,
    preview_windows: RefCell<Vec<Hwnd>>,
    /// Entry waiting for the hover delay before its list appears.
    preview_pending: Cell<Option<usize>>,
    preview_hovered: Cell<bool>,
    preview_show: Timer,
    preview_hide: Timer,
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
        ui.on_drag_started(|x| with_app(|a| a.dock.as_ref().map_or((), |d| d.drag_start(x))));
        ui.on_drag_moved(|x| with_app(|a| a.dock.as_ref().map_or((), |d| d.drag_move(a, x))));
        ui.on_drag_ended(|| with_app(|a| a.dock.as_ref().map_or((), |d| d.drag_end(a))));
        ui.on_preview_clicked(|i| with_app(|a| a.dock.as_ref().map_or((), |d| d.preview_clicked(i))));
        ui.on_preview_hover(|h| with_app(|a| a.dock.as_ref().map_or((), |d| d.preview_hover(h))));
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
            grown: Cell::new(LAYERED),
            anim: Timer::default(),
            bounces: RefCell::new(Vec::new()),
            bounce_anim: Timer::default(),
            dragging: Cell::new(None),
            width: Cell::new(400.0),
            preview: Cell::new(None),
            preview_windows: RefCell::new(Vec::new()),
            preview_pending: Cell::new(None),
            preview_hovered: Cell::new(false),
            preview_show: Timer::default(),
            preview_hide: Timer::default(),
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
        let preview = if LAYERED { PREVIEW_ROOM } else { 0.0 };
        Self::resting_height(m) + (m.max_icon_size() - m.icon_size) + TOOLTIP_ROOM + preview
    }

    fn window_rect(&self, app: &App, grown: bool) -> Rect {
        let mon = app.monitor.get();
        let m = Self::metrics(app);
        let h = if grown || LAYERED { Self::grown_height(&m) } else { Self::resting_height(&m) };
        let (w, h) = ((self.width.get() * mon.scale).round() as i32, (h * mon.scale).round() as i32);
        Rect { x: mon.bounds.x + (mon.bounds.w - w) / 2, y: mon.bounds.y + mon.bounds.h - h, w, h }
    }

    pub fn set_files_style(&self, style: FilesStyle) {
        self.ui.set_files_style(style.ui_index());
    }

    pub fn show(&self, app: &App) {
        self.set_files_style(FilesStyle::from_id(&app.cfg.borrow().dock.files_style));
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
        if self.hwnd.get().is_some() {
            ui::set_bounds(self.ui.window(), self.window_rect(app, self.grown.get()));
        }
        self.layout_icons(app);
    }

    fn layout_icons(&self, app: &App) {
        let m = Self::metrics(app);
        let kinds = self.kinds();
        let (slots, total) = layout_dock(&kinds, &m, self.width.get(), self.pointer.get(), self.strength.get());
        let lifts = self.lifts(m.icon_size);
        let plate_w = total + 2.0 * PADDING;
        self.ui.set_plate_x((self.width.get() - plate_w) / 2.0);
        self.ui.set_plate_width(plate_w);
        for (i, s) in slots.iter().enumerate() {
            let lift = lifts.get(i).copied().unwrap_or(0.0);
            if let Some(mut row) = self.model.row_data(i)
                && (row.x != s.x || row.size != s.size || row.lift != lift)
            {
                row.x = s.x;
                row.size = s.size;
                row.lift = lift;
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
        self.apply_region(app, &slots, &lifts, plate_w, hovered);
        *self.slots.borrow_mut() = slots;
        self.track_preview(app, hovered);
    }

    // ----- window list -------------------------------------------------

    /// Called on every layout: arms the hover delay for a running app,
    /// and lets the list go once the pointer has left icon and list.
    fn track_preview(&self, app: &App, hovered: Option<usize>) {
        if self.dragging.get().is_some() || !LAYERED || !app.cfg.borrow().dock.window_previews {
            return;
        }
        let wanted = hovered.filter(|i| {
            self.entries.borrow().get(*i).is_some_and(|e| e.special == Special::App && !e.windows.is_empty())
        });
        if wanted.is_some() && wanted == self.preview.get() {
            self.preview_hide.stop();
            return;
        }
        if wanted != self.preview_pending.get() {
            self.preview_pending.set(wanted);
            match wanted {
                Some(_) => self.preview_show.start(TimerMode::SingleShot, PREVIEW_DELAY, || {
                    with_app(|a| a.dock.as_ref().map_or((), |d| d.open_preview()))
                }),
                None => self.preview_show.stop(),
            }
        }
        if self.preview.get().is_some() && !self.preview_hide.running() {
            self.preview_hide.start(TimerMode::SingleShot, PREVIEW_GRACE, || {
                with_app(|a| a.dock.as_ref().map_or((), |d| d.maybe_close_preview()))
            });
        }
    }

    fn open_preview(&self) {
        let Some(i) = self.preview_pending.get() else { return };
        let entries = self.entries.borrow();
        let Some(e) = entries.get(i) else { return };
        let windows: Vec<Hwnd> = e.windows.iter().copied().take(PREVIEW_MAX).collect();
        if windows.is_empty() {
            return;
        }
        let titles: Vec<slint::SharedString> = windows
            .iter()
            .map(|h| {
                let t = windows_list::title(h.get());
                if t.trim().is_empty() { e.name.as_str().into() } else { t.into() }
            })
            .collect();
        let fg = windows_list::foreground();
        self.ui.set_preview_active(windows.iter().position(|h| *h == fg).map_or(-1, |p| p as i32));
        self.ui.set_preview_titles(Rc::new(VecModel::from(titles)).into());
        self.ui.set_preview_index(i as i32);
        *self.preview_windows.borrow_mut() = windows;
        self.preview.set(Some(i));
        self.preview_hide.stop();
    }

    fn maybe_close_preview(&self) {
        let still_on_icon =
            self.preview.get().is_some() && self.preview.get().map(|i| i as i32) == Some(self.ui.get_hovered());
        if !self.preview_hovered.get() && !still_on_icon {
            self.close_preview();
        }
    }

    fn close_preview(&self) {
        self.preview_show.stop();
        self.preview_hide.stop();
        self.preview.set(None);
        self.preview_pending.set(None);
        self.preview_hovered.set(false);
        self.preview_windows.borrow_mut().clear();
        self.ui.set_preview_index(-1);
    }

    fn preview_hover(&self, hovered: bool) {
        self.preview_hovered.set(hovered);
        if hovered {
            self.preview_hide.stop();
        } else if self.preview.get().is_some() {
            self.preview_hide.start(TimerMode::SingleShot, PREVIEW_GRACE, || {
                with_app(|a| a.dock.as_ref().map_or((), |d| d.maybe_close_preview()))
            });
        }
    }

    fn preview_clicked(&self, row: i32) {
        let target = self.preview_windows.borrow().get(row.max(0) as usize).copied();
        self.close_preview();
        if let Some(h) = target {
            windows_list::activate(h);
        }
    }

    /// Clips the window to plate + magnified icons + tooltip. This shapes the
    /// DWM backdrop and lets clicks outside the dock reach the windows below.
    fn apply_region(&self, app: &App, slots: &[DockSlot], lifts: &[f32], plate_w: f32, hovered: Option<usize>) {
        if LAYERED {
            return;
        }
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
        for (s, lift) in slots.iter().zip(lifts) {
            let top = icons_bottom - s.size - lift;
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
            let ty = (icons_bottom - s.size - th - 8.0).max(0.0);
            parts.push(window::RegionPart {
                rect: Rect { x: px(tx), y: px(ty), w: px(tw), h: px(th) },
                radius: px(th / 2.0),
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
        if hovered && !self.grown.replace(true) && self.hwnd.get().is_some() {
            ui::set_bounds(self.ui.window(), self.window_rect(app, true));
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
                self.shrink_if_idle(app);
            }
        }
    }

    /// Back to the resting strip once nothing is magnified or bouncing.
    fn shrink_if_idle(&self, app: &App) {
        if !LAYERED
            && self.strength.get() == 0.0
            && self.bounces.borrow().is_empty()
            && self.grown.replace(false)
            && self.hwnd.get().is_some()
        {
            ui::set_bounds(self.ui.window(), self.window_rect(app, false));
        }
        self.layout_icons(app);
    }

    /// Per-slot bounce offsets, in entry order.
    fn lifts(&self, icon_size: f32) -> Vec<f32> {
        let bounces = self.bounces.borrow();
        self.entries
            .borrow()
            .iter()
            .map(|e| {
                bounces.iter().find(|b| b.app_id.eq_ignore_ascii_case(&e.app_id)).map_or(0.0, |b| b.lift(icon_size))
            })
            .collect()
    }

    /// Starts the launch bounce for `app_id` if it has an icon in the dock.
    pub fn bounce(&self, app: &App, app_id: &str) {
        let running = {
            let entries = self.entries.borrow();
            let Some(e) = entries.iter().find(|e| !e.separator && e.app_id.eq_ignore_ascii_case(app_id)) else {
                return;
            };
            !e.windows.is_empty()
        };
        {
            let mut bounces = self.bounces.borrow_mut();
            bounces.retain(|b| !b.app_id.eq_ignore_ascii_case(app_id));
            // A new instance of a running app gets a single hop.
            let hops = if running { 1 } else { BOUNCE_MAX_HOPS };
            bounces.push(Bounce { app_id: app_id.to_owned(), start: Instant::now(), hops });
        }
        if !self.grown.replace(true) && self.hwnd.get().is_some() {
            ui::set_bounds(self.ui.window(), self.window_rect(app, true));
        }
        self.bounce_anim.start(TimerMode::Repeated, Duration::from_millis(16), || {
            with_app(|a| {
                if let Some(d) = a.dock.as_ref() {
                    d.bounce_tick(a);
                }
            })
        });
    }

    fn bounce_tick(&self, app: &App) {
        self.bounces.borrow_mut().retain(|b| !b.done());
        if self.bounces.borrow().is_empty() {
            self.bounce_anim.stop();
            self.shrink_if_idle(app);
        } else {
            self.layout_icons(app);
        }
    }

    /// Re-reads pinned items and running windows.
    pub fn rebuild(&self, app: &App) {
        let show_running = app.cfg.borrow().dock.show_running;
        let custom_icons = app.cfg.borrow().dock.custom_icons;
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
                    special: Special::App,
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
                    special: Special::App,
                    windows: vec![w.hwnd],
                });
            }
            if !running.is_empty() && !entries.is_empty() {
                entries.push(Entry::separator());
            }
            entries.extend(running);
        }
        entries.insert(0, Entry::builtin(Special::Launcher, "Inicio"));
        entries.push(Entry::separator());
        entries.push(Entry::builtin(Special::Trash, "Papelera"));

        for e in entries.iter().filter(|e| !e.separator && e.special == Special::App && !e.icon_path.exists()) {
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
                lift: 0.0,
                kind: match e.special {
                    Special::App if custom_icons => system_skin(&e.app_id, &e.exe_path),
                    s => s.ui_kind(),
                },
                initial: e.name.chars().next().map(|c| c.to_uppercase().collect::<String>()).unwrap_or_default().into(),
            })
            .collect();
        let structure_changed = {
            let old = self.entries.borrow();
            old.len() != entries.len()
                || old.iter().zip(&entries).any(|(a, b)| a.app_id != b.app_id || a.special != b.special)
        };
        for b in self.bounces.borrow_mut().iter_mut() {
            if entries.iter().any(|e| !e.windows.is_empty() && e.app_id.eq_ignore_ascii_case(&b.app_id)) {
                // Finish the hop in flight, then land.
                b.hops = b.hops.min(b.elapsed_hops().floor() as u32 + 1);
            }
        }
        *self.entries.borrow_mut() = entries;
        self.model.set_vec(rows);
        if structure_changed {
            // Indices moved: a list on screen may point at another app now.
            self.close_preview();
            self.relayout(app);
        } else {
            self.layout_icons(app);
        }
    }

    /// Screen rectangle (physical px) of the dock icon that owns `hwnd`,
    /// matched by window handle or, for a window the dock has not listed yet,
    /// by AppUserModelID / executable.
    pub fn icon_rect_for(&self, app: &App, hwnd: Hwnd) -> Option<Rect> {
        let index = {
            let entries = self.entries.borrow();
            entries.iter().position(|e| e.windows.contains(&hwnd)).or_else(|| {
                let aumid = windows_list::window_aumid(hwnd.get());
                let exe = windows_list::process_path(windows_list::process_id(hwnd.get()));
                entries.iter().position(|e| {
                    !e.separator
                        && (aumid.as_deref().is_some_and(|a| a.eq_ignore_ascii_case(&e.app_id))
                            || (!exe.is_empty()
                                && (exe.eq_ignore_ascii_case(&e.exe_path) || exe.eq_ignore_ascii_case(&e.app_id))))
                })
            })?
        };
        let slot = self.slots.borrow().get(index).copied()?;
        let scale = app.monitor.get().scale;
        let win = self.window_rect(app, true);
        let icons_bottom = win.h as f32 / scale - BOTTOM_MARGIN - PADDING;
        let px = |v: f32| (v * scale).round() as i32;
        Some(Rect {
            x: win.x + px(slot.x),
            y: win.y + px(icons_bottom - slot.size),
            w: px(slot.size),
            h: px(slot.size),
        })
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

    /// Pinned app icons can be dragged to a new place among the pinned ones.
    fn drag_start(&self, x: f32) {
        self.close_preview();
        log::debug!("dock drag start x={x} hit={:?}", hit_test(&self.slots.borrow(), x));
        let Some(i) = hit_test(&self.slots.borrow(), x) else { return };
        let movable = self.entries.borrow().get(i).is_some_and(|e| e.pinned && e.special == Special::App);
        if movable {
            self.dragging.set(Some(i));
            self.ui.set_drag_index(i as i32);
        }
    }

    fn drag_move(&self, app: &App, x: f32) {
        let Some(from) = self.dragging.get() else { return };
        let Some(to) = hit_test(&self.slots.borrow(), x) else { return };
        let target_ok = self.entries.borrow().get(to).is_some_and(|e| e.pinned && e.special == Special::App);
        if to == from || !target_ok {
            return;
        }
        {
            let mut entries = self.entries.borrow_mut();
            let e = entries.remove(from);
            entries.insert(to, e);
        }
        let row = self.model.remove(from);
        self.model.insert(to, row);
        self.dragging.set(Some(to));
        self.ui.set_drag_index(to as i32);
        self.layout_icons(app);
    }

    fn drag_end(&self, app: &App) {
        log::debug!("dock drag end {:?}", self.dragging.get());
        if self.dragging.take().is_none() {
            return;
        }
        self.ui.set_drag_index(-1);
        let ids: Vec<String> = self.entries.borrow().iter().filter(|e| e.pinned).map(|e| e.app_id.clone()).collect();
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        if let Err(e) = app.store.borrow_mut().reorder_dock(&ids) {
            log::warn!("saving dock order: {e}");
        }
    }

    fn on_click(&self, app: &App, x: f32, click: Click) {
        let Some(index) = hit_test(&self.slots.borrow(), x) else { return };
        let Some(entry) = self.entries.borrow().get(index).cloned() else { return };
        if entry.separator {
            return;
        }
        match (entry.special, click) {
            (Special::Launcher, Click::Primary) => return app.launcher.toggle(app),
            (Special::Launcher, Click::Secondary) => return app.settings.toggle(app),
            (Special::Trash, Click::Primary) => return app.launch(TRASH),
            (Special::App, _) => {}
            _ => return,
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

#[cfg(test)]
mod tests {
    use super::FilesStyle;

    #[test]
    fn files_styles_cycle_and_parse() {
        let mut s = FilesStyle::Classic;
        let mut seen = Vec::new();
        for _ in 0..5 {
            seen.push(s.id());
            assert_eq!(FilesStyle::from_id(s.id()), s);
            s = s.next();
        }
        assert_eq!(s, FilesStyle::Classic);
        assert_eq!(seen, ["classic", "dot", "mono", "theme", "glass"]);
        assert_eq!(FilesStyle::from_id("nope"), FilesStyle::Classic);
    }
}
