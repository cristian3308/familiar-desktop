//! W3: desktop widgets drawn as dot-matrix frames (SIGNAL/01, CLOCK/GRID).
//!
//! Each widget is its own small window kept at the bottom of the z-order
//! (or parented to the wallpaper layer), so the desktop and its icons stay
//! clickable everywhere else. Widgets can be dragged; on release they snap
//! to a grid and the position is saved in SQLite.

use std::cell::Cell;

use familiar_layout::WidgetGrid;
use familiar_signal::Matrix;
use familiar_signal::raster::{self, DotStyle, Rgba};
use familiar_store::WidgetLayout;
use familiar_sys::window::{self, Rect, SurfaceOptions};
use familiar_sys::{Hwnd, clock};
use slint::ComponentHandle;

use crate::app::{App, with_app};
use crate::ui::{self, WidgetWindow};

const CELL: f32 = 88.0;
const GAP: f32 = 12.0;
const MARGIN: f32 = 24.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Signal,
    Clock,
}

impl Kind {
    fn from_id(id: &str) -> Option<Self> {
        match id {
            "signal_01" => Some(Self::Signal),
            "clock_grid" => Some(Self::Clock),
            _ => None,
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Signal => "signal_01",
            Self::Clock => "clock_grid",
        }
    }

    /// Span in grid units.
    fn span(self) -> (i32, i32) {
        match self {
            Self::Signal => (3, 2),
            Self::Clock => (3, 1),
        }
    }

    fn caption(self) -> &'static str {
        match self {
            Self::Signal => "SIGNAL/01",
            Self::Clock => "CLOCK/GRID",
        }
    }
}

pub struct Widget {
    pub kind: Kind,
    pub ui: WidgetWindow,
    hwnd: Cell<Option<Hwnd>>,
    /// Top-left in logical px relative to the work area.
    pos: Cell<(f32, f32)>,
    cell: Cell<(i32, i32)>,
}

pub struct Widgets {
    pub items: Vec<Widget>,
}

impl Widgets {
    pub fn new(app_ids: &[String]) -> Result<Self, slint::PlatformError> {
        let mut items = Vec::new();
        for (index, id) in app_ids.iter().enumerate() {
            let Some(kind) = Kind::from_id(id) else {
                log::warn!("unknown widget {id:?}");
                continue;
            };
            let ui = WidgetWindow::new()?;
            ui.set_caption(kind.caption().into());
            ui.on_drag_moved(move |dx, dy| {
                with_app(|a| {
                    if let Some(w) = a.widgets.as_ref().and_then(|w| w.items.get(index)) {
                        w.drag(a, dx, dy);
                    }
                })
            });
            ui.on_drag_ended(move || {
                with_app(|a| {
                    if let Some(w) = a.widgets.as_ref().and_then(|w| w.items.get(index)) {
                        w.drop_at_grid(a);
                    }
                })
            });
            items.push(Widget { kind, ui, hwnd: Cell::new(None), pos: Cell::new((0.0, 0.0)), cell: Cell::new((0, 0)) });
        }
        Ok(Self { items })
    }

    pub fn grid(app: &App) -> WidgetGrid {
        let mon = app.monitor.get();
        let work = mon.work;
        WidgetGrid::for_area(work.w as f32 / mon.scale, work.h as f32 / mon.scale, CELL, GAP, MARGIN)
    }

    pub fn show(&self, app: &App) {
        let grid = Self::grid(app);
        let saved = app.store.borrow().widget_layouts("desktop").unwrap_or_default();
        let mut next_row = 0;
        for w in &self.items {
            let (sw, sh) = w.kind.span();
            let cell = match saved.iter().find(|l| l.id == w.kind.id()) {
                Some(l) => (l.grid_x.clamp(0, (grid.columns - sw).max(0)), l.grid_y.clamp(0, (grid.rows - sh).max(0))),
                None => {
                    // Default: stacked along the right edge, under the bar.
                    let c = ((grid.columns - sw).max(0), next_row);
                    next_row += sh;
                    c
                }
            };
            w.cell.set(cell);
            w.pos.set(grid.to_pixels(cell.0, cell.1));
            w.show(app, &grid);
        }
        self.tick_clock(app);
        self.render_signal(app, &[]);
    }

    pub fn relayout(&self, app: &App) {
        let grid = Self::grid(app);
        for w in &self.items {
            let (cx, cy) = w.cell.get();
            let (sw, sh) = w.kind.span();
            let cell = (cx.clamp(0, (grid.columns - sw).max(0)), cy.clamp(0, (grid.rows - sh).max(0)));
            w.cell.set(cell);
            w.pos.set(grid.to_pixels(cell.0, cell.1));
            w.place(app, &grid);
        }
    }

    pub fn apply_theme(&self, app: &App) {
        let t = app.theme.borrow();
        for w in &self.items {
            ui::push_theme(&w.ui, &t);
            if let Some(h) = w.hwnd.get() {
                ui::apply_material(h, &t, false);
                ui::round_region(h, w.ui.window(), t.metrics.corner_radius as f32);
            }
        }
        drop(t);
        self.tick_clock(app);
        self.render_signal(app, &[]);
    }

    pub fn set_visible(&self, visible: bool) {
        for w in &self.items {
            ui::set_native_visible(w.hwnd.get(), visible);
        }
    }

    pub fn has(&self, kind: Kind) -> bool {
        self.items.iter().any(|w| w.kind == kind)
    }

    fn style(app: &App, pitch: u32) -> DotStyle {
        let t = app.theme.borrow();
        let c = |c: familiar_theme::Color| Rgba(c.r, c.g, c.b, c.a);
        DotStyle { pitch, radius: 0.36, on: c(t.colors.accent), off: c(t.colors.dot_off) }
    }

    pub fn tick_clock(&self, app: &App) {
        let Some(w) = self.items.iter().find(|w| w.kind == Kind::Clock) else { return };
        let now = clock::now();
        let mut m = Matrix::new(29, 7);
        m.draw_text(&format!("{:02}:{:02}", now.hour, now.minute), 0, 0, 1.0);
        let (iw, ih, px) = raster::render(&m, &Self::style(app, 8));
        w.ui.set_matrix(ui::image_from_rgba(iw, ih, &px));
        w.ui.set_subcaption(clock::date_string("ddd d").to_uppercase().into());
    }

    /// Renders SIGNAL/01 from the current spectrum (empty = idle face).
    pub fn render_signal(&self, app: &App, bands: &[f32]) {
        let Some(w) = self.items.iter().find(|w| w.kind == Kind::Signal) else { return };
        let mut m = Matrix::new(32, 16);
        m.draw_signal_face(bands);
        let (iw, ih, px) = raster::render(&m, &Self::style(app, 7));
        w.ui.set_matrix(ui::image_from_rgba(iw, ih, &px));
        let live = bands.iter().any(|b| *b > 0.02);
        w.ui.set_subcaption(if live { "LIVE" } else { "IDLE" }.into());
    }
}

impl Widget {
    fn rect(&self, app: &App, grid: &WidgetGrid) -> Rect {
        let mon = app.monitor.get();
        let (x, y) = self.pos.get();
        let (sw, sh) = self.kind.span();
        let (w, h) = grid.span_pixels(sw, sh);
        Rect {
            x: mon.work.x + (x * mon.scale).round() as i32,
            y: mon.work.y + (y * mon.scale).round() as i32,
            w: (w * mon.scale).round() as i32,
            h: (h * mon.scale).round() as i32,
        }
    }

    fn show(&self, app: &App, grid: &WidgetGrid) {
        let r = self.rect(app, grid);
        let win = self.ui.window();
        win.set_position(slint::PhysicalPosition::new(r.x, r.y));
        win.set_size(slint::PhysicalSize::new(r.w as u32, r.h as u32));
        if let Err(e) = self.ui.show() {
            log::error!("widget {}: {e}", self.kind.id());
            return;
        }
        let Some(hwnd) = ui::hwnd_of(win) else { return };
        self.hwnd.set(Some(hwnd));
        window::apply_surface(
            hwnd.get(),
            SurfaceOptions { tool_window: true, no_activate: true, topmost: false, click_through: false },
        );
        ui::apply_material(hwnd, &app.theme.borrow(), false);
        ui::round_region(hwnd, win, app.theme.borrow().metrics.corner_radius as f32);
        let attached =
            app.cfg.borrow().widgets.attach_to_wallpaper && familiar_sys::desktop::attach_to_wallpaper(hwnd.get());
        if attached {
            // Child coordinates are relative to the wallpaper host, which
            // starts at the virtual-screen origin; re-place in those terms.
            self.place(app, grid);
        } else {
            window::pin_to_bottom(hwnd.get());
        }
    }

    fn place(&self, app: &App, grid: &WidgetGrid) {
        if let Some(h) = self.hwnd.get() {
            window::set_bounds(h.get(), self.rect(app, grid));
        }
    }

    fn drag(&self, app: &App, dx: f32, dy: f32) {
        let (x, y) = self.pos.get();
        self.pos.set((x + dx, y + dy));
        self.place(app, &Widgets::grid(app));
    }

    fn drop_at_grid(&self, app: &App) {
        let grid = Widgets::grid(app);
        let (x, y) = self.pos.get();
        let (sw, sh) = self.kind.span();
        let cell = grid.snap(x, y, sw, sh);
        self.cell.set(cell);
        self.pos.set(grid.to_pixels(cell.0, cell.1));
        self.place(app, &grid);
        let layout = WidgetLayout {
            id: self.kind.id().into(),
            target_screen: "desktop".into(),
            grid_x: cell.0,
            grid_y: cell.1,
            width_units: sw,
            height_units: sh,
            custom_config_json: None,
        };
        if let Err(e) = app.store.borrow().upsert_widget(&layout) {
            log::warn!("saving widget layout: {e}");
        }
    }
}
