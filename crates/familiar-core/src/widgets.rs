//! W3: desktop widgets: macOS-style cards (calendar, weather, system) and
//! dot-matrix frames (SIGNAL/01, CLOCK/GRID).
//!
//! Each widget is its own small window kept at the bottom of the z-order
//! (or parented to the wallpaper layer), so the desktop and its icons stay
//! clickable everywhere else. Widgets can be dragged; on release they snap
//! to a grid and the position is saved in SQLite.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use familiar_layout::WidgetGrid;
use familiar_signal::Matrix;
use familiar_signal::raster::{self, DotStyle, Rgba};
use familiar_signal::scenes::{self, Scene, SceneState};
use familiar_signal::signal::{self, SignalMode, SignalState};
use familiar_store::WidgetLayout;
use familiar_sys::media::{MediaCommand, NowPlaying};
use familiar_sys::sysinfo::{self, CpuMeter};
use familiar_sys::weather::{self, Sky, Weather};
use familiar_sys::window::{self, Rect, SurfaceOptions};
use familiar_sys::{Hwnd, clock, power};
use slint::{ComponentHandle, Timer, TimerMode, VecModel};

use crate::app::{App, post, with_app};
use crate::config::AppConfig;
use crate::ui::{self, CalCell, WeatherHour, WidgetWindow};

const CELL: f32 = 88.0;
const GAP: f32 = 12.0;
const MARGIN: f32 = 24.0;
const WEATHER_REFRESH: Duration = Duration::from_secs(30 * 60);
const SYSTEM_REFRESH: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Signal,
    Clock,
    Calendar,
    Weather,
    System,
    Visualizer,
    AnalogClock,
    NowPlaying,
}

impl Kind {
    fn from_id(id: &str) -> Option<Self> {
        match id {
            "signal_01" => Some(Self::Signal),
            "clock_grid" => Some(Self::Clock),
            "calendar" => Some(Self::Calendar),
            "weather" => Some(Self::Weather),
            "system" => Some(Self::System),
            "visualizer" => Some(Self::Visualizer),
            "analog_clock" => Some(Self::AnalogClock),
            "now_playing" => Some(Self::NowPlaying),
            _ => None,
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Signal => "signal_01",
            Self::Clock => "clock_grid",
            Self::Calendar => "calendar",
            Self::Weather => "weather",
            Self::System => "system",
            Self::Visualizer => "visualizer",
            Self::AnalogClock => "analog_clock",
            Self::NowPlaying => "now_playing",
        }
    }

    /// Span in grid units.
    fn span(self) -> (i32, i32) {
        match self {
            Self::Signal => (3, 2),
            Self::Clock => (3, 1),
            Self::Calendar | Self::Weather => (4, 2),
            Self::System | Self::NowPlaying => (4, 1),
            Self::Visualizer => (4, 2),
            Self::AnalogClock => (2, 2),
        }
    }

    fn caption(self) -> &'static str {
        match self {
            Self::Signal => "SIGNAL/01",
            Self::Clock => "CLOCK/GRID",
            Self::Visualizer => "VISUALIZER",
            Self::AnalogClock => "CLOCK/ANALOG",
            _ => "",
        }
    }

    /// Matches `WidgetWindow.kind` in `widgets.slint`.
    fn ui_kind(self) -> i32 {
        match self {
            Self::Signal | Self::Clock | Self::Visualizer | Self::AnalogClock => 0,
            Self::Calendar => 1,
            Self::Weather => 2,
            Self::System => 3,
            Self::NowPlaying => 4,
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
    pub items: RefCell<Vec<Rc<Widget>>>,
    weather_timer: Timer,
    system_timer: Timer,
    cpu: RefCell<CpuMeter>,
    scene: std::cell::Cell<usize>,
    scene_state: RefCell<SceneState>,
    signal_state: RefCell<SignalState>,
    /// Keeps SIGNAL/01 and the visualizer moving while no audio arrives.
    idle_anim: Timer,
    started: std::time::Instant,
    /// When the last spectrum frame arrived (audio frames drive rendering).
    last_audio: Cell<Option<std::time::Instant>>,
}

/// The configured SIGNAL/01 mode (face when unknown).
pub fn signal_mode(cfg: &AppConfig) -> SignalMode {
    SignalMode::from_id(&cfg.widgets.signal_mode).unwrap_or(SignalMode::Face)
}

const IDLE_FRAME: Duration = Duration::from_millis(50);

impl Widgets {
    pub fn new(app_ids: &[String]) -> Result<Self, slint::PlatformError> {
        let mut items = Vec::new();
        for id in app_ids {
            match Kind::from_id(id) {
                Some(kind) if !items.iter().any(|w: &Rc<Widget>| w.kind == kind) => items.push(Self::build(kind)?),
                Some(_) => {}
                None => log::warn!("unknown widget {id:?}"),
            }
        }
        Ok(Self {
            items: RefCell::new(items),
            weather_timer: Timer::default(),
            system_timer: Timer::default(),
            cpu: RefCell::default(),
            scene: std::cell::Cell::new(0),
            scene_state: RefCell::default(),
            signal_state: RefCell::default(),
            idle_anim: Timer::default(),
            started: std::time::Instant::now(),
            last_audio: Cell::new(None),
        })
    }

    fn build(kind: Kind) -> Result<Rc<Widget>, slint::PlatformError> {
        let ui = WidgetWindow::new()?;
        ui.set_caption(kind.caption().into());
        ui.set_kind(kind.ui_kind());
        ui.on_drag_moved(move |dx, dy| {
            with_app(|a| {
                if let Some(w) = a.widgets.as_ref().and_then(|ws| ws.get(kind)) {
                    w.drag(a, dx, dy);
                }
            })
        });
        ui.on_tapped(move || {
            with_app(|a| {
                if let Some(ws) = a.widgets.as_ref() {
                    match kind {
                        Kind::Visualizer => ws.next_scene(a),
                        Kind::Signal => ws.next_signal_mode(a),
                        _ => {}
                    }
                }
            })
        });
        ui.on_np_previous(|| with_app(|a| a.media_command(MediaCommand::Previous)));
        ui.on_np_play_pause(|| with_app(|a| a.media_command(MediaCommand::PlayPause)));
        ui.on_np_next(|| with_app(|a| a.media_command(MediaCommand::Next)));
        ui.on_drag_ended(move || {
            with_app(|a| {
                if let Some(w) = a.widgets.as_ref().and_then(|ws| ws.get(kind)) {
                    w.drop_at_grid(a);
                }
            })
        });
        Ok(Rc::new(Widget { kind, ui, hwnd: Cell::new(None), pos: Cell::new((0.0, 0.0)), cell: Cell::new((0, 0)) }))
    }

    fn get(&self, kind: Kind) -> Option<Rc<Widget>> {
        self.items.borrow().iter().find(|w| w.kind == kind).cloned()
    }

    /// Adds and removes widgets to match `ids` while running (Settings).
    pub fn sync(&self, app: &App, ids: &[String]) {
        let wanted: Vec<Kind> = ids.iter().filter_map(|id| Kind::from_id(id)).collect();
        let removed: Vec<Rc<Widget>> = {
            let mut items = self.items.borrow_mut();
            let (keep, gone): (Vec<_>, Vec<_>) = items.drain(..).partition(|w| wanted.contains(&w.kind));
            *items = keep;
            gone
        };
        for w in removed {
            let _ = w.ui.hide();
        }
        for kind in wanted {
            if self.get(kind).is_none() {
                match Self::build(kind) {
                    Ok(w) => {
                        ui::push_theme(&w.ui, &app.theme.borrow());
                        self.items.borrow_mut().push(w);
                    }
                    Err(e) => log::warn!("widget {}: {e}", kind.id()),
                }
            }
        }
        self.show(app);
        if let Some(m) = app.media() {
            self.set_media(Some(&m));
        }
    }

    pub fn grid(app: &App) -> WidgetGrid {
        let mon = app.monitor.get();
        let work = mon.work;
        WidgetGrid::for_area(work.w as f32 / mon.scale, work.h as f32 / mon.scale, CELL, GAP, MARGIN)
    }

    pub fn show(&self, app: &App) {
        let grid = Self::grid(app);
        let saved = app.store.borrow().widget_layouts("desktop").unwrap_or_default();
        // Saved widgets keep their cell; new ones take the first free spot,
        // scanning columns from the right edge and rows from the top.
        let mut taken: Vec<(i32, i32, i32, i32)> = Vec::new();
        let overlaps = |taken: &[(i32, i32, i32, i32)], x: i32, y: i32, w: i32, h: i32| {
            taken.iter().any(|&(tx, ty, tw, th)| x < tx + tw && tx < x + w && y < ty + th && ty < y + h)
        };
        let items = self.items.borrow().clone();
        let mut cells = vec![None; items.len()];
        for (i, w) in items.iter().enumerate() {
            let (sw, sh) = w.kind.span();
            // Already on screen: keep it where it is.
            if w.hwnd.get().is_some() {
                let c = w.cell.get();
                taken.push((c.0, c.1, sw, sh));
                cells[i] = Some(c);
            } else if let Some(l) = saved.iter().find(|l| l.id == w.kind.id()) {
                let c = (l.grid_x.clamp(0, (grid.columns - sw).max(0)), l.grid_y.clamp(0, (grid.rows - sh).max(0)));
                taken.push((c.0, c.1, sw, sh));
                cells[i] = Some(c);
            }
        }
        for (i, w) in items.iter().enumerate() {
            if cells[i].is_some() {
                continue;
            }
            let (sw, sh) = w.kind.span();
            let mut spot = ((grid.columns - sw).max(0), 0);
            'search: for x in (0..=(grid.columns - sw).max(0)).rev() {
                for y in 0..=(grid.rows - sh).max(0) {
                    if !overlaps(&taken, x, y, sw, sh) {
                        spot = (x, y);
                        break 'search;
                    }
                }
            }
            taken.push((spot.0, spot.1, sw, sh));
            cells[i] = Some(spot);
        }
        for (w, cell) in items.iter().zip(cells) {
            if w.hwnd.get().is_some() {
                continue;
            }
            let cell = cell.unwrap_or_default();
            w.cell.set(cell);
            w.pos.set(grid.to_pixels(cell.0, cell.1));
            w.show(app, &grid);
        }
        self.tick_clock(app);
        self.render_signal(app, &[]);
        self.render_visualizer(app, &[]);
        self.start_live(app);
    }

    /// Starts the periodic refreshes the visible widgets need.
    fn start_live(&self, app: &App) {
        let animated = self.has(Kind::Signal) || self.has(Kind::Visualizer);
        if !animated {
            self.idle_anim.stop();
        } else if !self.idle_anim.running() {
            self.idle_anim.start(TimerMode::Repeated, IDLE_FRAME, || {
                with_app(|a| {
                    if let Some(w) = &a.widgets {
                        w.idle_frame(a);
                    }
                })
            });
        }
        if !self.has(Kind::Weather) {
            self.weather_timer.stop();
        } else if !self.weather_timer.running() {
            self.refresh_weather(app);
            self.weather_timer.start(TimerMode::Repeated, WEATHER_REFRESH, || {
                with_app(|a| {
                    if let Some(w) = &a.widgets {
                        w.refresh_weather(a);
                    }
                })
            });
        }
        if !self.has(Kind::System) {
            self.system_timer.stop();
        } else if !self.system_timer.running() {
            self.refresh_system();
            self.system_timer.start(TimerMode::Repeated, SYSTEM_REFRESH, || {
                with_app(|a| {
                    if let Some(w) = &a.widgets {
                        w.refresh_system();
                    }
                })
            });
        }
    }

    /// Fetches the forecast on a background thread.
    pub fn refresh_weather(&self, app: &App) {
        let city = app.cfg.borrow().widgets.weather_city.clone();
        let spawned = std::thread::Builder::new().name("weather".into()).spawn(move || match weather::fetch(&city) {
            Ok(w) => post(move |a| {
                if let Some(ws) = &a.widgets {
                    ws.set_weather(&w);
                }
            }),
            Err(e) => log::warn!("weather for {city:?}: {e}"),
        });
        if let Err(e) = spawned {
            log::warn!("weather thread: {e}");
        }
    }

    fn set_weather(&self, w: &Weather) {
        let Some(widget) = self.get(Kind::Weather) else { return };
        let ui = &widget.ui;
        ui.set_w_ready(true);
        ui.set_w_city(w.city.as_str().into());
        ui.set_w_temp(format!("{:.0}°", w.temp).into());
        ui.set_w_desc(Sky::from_code(w.code).describe_es().into());
        ui.set_w_hilo(format!("Máx {:.0}°  Mín {:.0}°", w.high, w.low).into());
        ui.set_w_sky(sky_index(w.code));
        ui.set_w_day(w.is_day);
        let hours: Vec<WeatherHour> = w
            .hours
            .iter()
            .enumerate()
            .map(|(i, h)| WeatherHour {
                label: if i == 0 { "Ahora".into() } else { hour_label(h.hour).into() },
                temp: format!("{:.0}°", h.temp).into(),
                sky: sky_index(h.code),
                day: if i == 0 { w.is_day } else { (6..18).contains(&h.hour) },
            })
            .collect();
        ui.set_w_hours(Rc::new(VecModel::from(hours)).into());
    }

    fn refresh_system(&self) {
        let Some(widget) = self.get(Kind::System) else { return };
        let ui = &widget.ui;
        ui.set_s_cpu(self.cpu.borrow_mut().sample() as i32);
        let (pct, used, total) = sysinfo::memory();
        ui.set_s_ram(pct as i32);
        ui.set_s_ram_detail(format!("{used:.1} / {total:.0} GB").into());
        let b = power::battery();
        ui.set_s_battery(b.percent.map(i32::from).unwrap_or(-1));
        ui.set_s_charging(b.charging);
    }

    fn render_calendar(&self) {
        let Some(widget) = self.get(Kind::Calendar) else { return };
        let now = clock::now();
        let ui = &widget.ui;
        ui.set_cal_weekday(clock::date_string("dddd").to_uppercase().into());
        ui.set_cal_day(now.day.to_string().into());
        ui.set_cal_month(capitalize(&clock::date_string("MMMM yyyy")).into());
        let cells: Vec<CalCell> = month_grid(now.year, now.month, now.day, now.weekday)
            .into_iter()
            .map(|(d, today, muted)| CalCell { label: d.to_string().into(), today, muted })
            .collect();
        ui.set_cal_cells(Rc::new(VecModel::from(cells)).into());
    }

    pub fn relayout(&self, app: &App) {
        let grid = Self::grid(app);
        for w in self.items.borrow().iter() {
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
        for w in self.items.borrow().iter() {
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
        for w in self.items.borrow().iter() {
            ui::set_native_visible(w.hwnd.get(), visible);
        }
        // Nothing to look at in calm mode: stop sampling the CPU.
        if visible {
            if self.has(Kind::System) && !self.system_timer.running() {
                self.system_timer.restart();
            }
        } else {
            self.system_timer.stop();
        }
    }

    /// The desktop came to the front (or left it): keep widgets visible
    /// above it, and back at the bottom once an app is active again.
    pub fn desktop_in_front(&self, front: bool) {
        window::set_pinned_float(front);
        for w in self.items.borrow().iter() {
            if let Some(h) = w.hwnd.get() {
                window::restack_pinned(h.get());
            }
        }
    }

    pub fn has(&self, kind: Kind) -> bool {
        self.items.borrow().iter().any(|w| w.kind == kind)
    }

    fn style(app: &App, pitch: u32) -> DotStyle {
        let t = app.theme.borrow();
        let c = |c: familiar_theme::Color| Rgba(c.r, c.g, c.b, c.a);
        DotStyle { pitch, radius: 0.36, on: c(t.colors.accent), off: c(t.colors.dot_off) }
    }

    pub fn tick_clock(&self, app: &App) {
        self.render_calendar();
        self.render_analog(app);
        let Some(w) = self.get(Kind::Clock) else { return };
        let now = clock::now();
        let mut m = Matrix::new(29, 7);
        m.draw_text(&format!("{:02}:{:02}", now.hour, now.minute), 0, 0, 1.0);
        let (iw, ih, px) = raster::render(&m, &Self::style(app, 8));
        w.ui.set_matrix(ui::image_from_rgba(iw, ih, &px));
        w.ui.set_subcaption(clock::date_string("ddd d").to_uppercase().into());
    }

    /// A spectrum frame from the audio thread: it drives both animations.
    pub fn on_audio(&self, app: &App, bands: &[f32]) {
        self.last_audio.set(Some(std::time::Instant::now()));
        self.render_signal(app, bands);
        self.render_visualizer(app, bands);
    }

    /// Without audio frames, keep the animations alive on a slow timer.
    fn idle_frame(&self, app: &App) {
        if self.last_audio.get().is_some_and(|t| t.elapsed() < IDLE_FRAME * 2) {
            return;
        }
        self.render_signal(app, &[]);
        if self.get(Kind::Visualizer).is_some() {
            self.render_visualizer(app, &[]);
        }
    }

    /// Renders SIGNAL/01 in its configured mode (empty bands = silence).
    pub fn render_signal(&self, app: &App, bands: &[f32]) {
        let Some(w) = self.get(Kind::Signal) else { return };
        let mode = signal_mode(&app.cfg.borrow());
        let now = clock::now();
        let frame = signal::Frame {
            t: self.started.elapsed().as_secs_f32(),
            hour: now.hour.into(),
            minute: now.minute.into(),
            second: now.second.into(),
        };
        let mut m = Matrix::new(32, 16);
        signal::draw(&mut m, mode, bands, frame, &mut self.signal_state.borrow_mut());
        let (iw, ih, px) = raster::render(&m, &Self::style(app, 7));
        w.ui.set_matrix(ui::image_from_rgba(iw, ih, &px));
        let live = bands.iter().any(|b| *b > 0.02);
        w.ui.set_subcaption(format!("{} · {}", mode.name(), if live { "LIVE" } else { "IDLE" }).into());
    }

    /// Next SIGNAL/01 mode (tap on the widget or Settings), saved to config.
    pub fn next_signal_mode(&self, app: &App) {
        let next = signal_mode(&app.cfg.borrow()).next();
        app.cfg.borrow_mut().widgets.signal_mode = next.id().into();
        app.save_config();
        *self.signal_state.borrow_mut() = SignalState::default();
        self.render_signal(app, &[]);
        app.settings.refresh_signal_mode(next);
    }

    /// Widgets that need the audio spectrum or media sessions.
    pub fn wants_media(&self) -> bool {
        self.items.borrow().iter().any(|w| matches!(w.kind, Kind::Signal | Kind::Visualizer | Kind::NowPlaying))
    }

    /// Draws the current visualizer scene (empty bands = resting frame).
    pub fn render_visualizer(&self, app: &App, bands: &[f32]) {
        let Some(w) = self.get(Kind::Visualizer) else { return };
        let scene = Scene::ALL[self.scene.get() % Scene::ALL.len()];
        let mut m = Matrix::new(48, 20);
        scenes::draw(&mut m, scene, bands, &mut self.scene_state.borrow_mut());
        let (iw, ih, px) = raster::render(&m, &Self::style(app, 7));
        w.ui.set_matrix(ui::image_from_rgba(iw, ih, &px));
        let n = Scene::ALL.len();
        w.ui.set_subcaption(format!("{:02}/{n:02} {}", self.scene.get() % n + 1, scene.name()).into());
    }

    fn next_scene(&self, app: &App) {
        self.scene.set((self.scene.get() + 1) % Scene::ALL.len());
        *self.scene_state.borrow_mut() = SceneState::default();
        self.render_visualizer(app, &[]);
    }

    /// Dot-matrix analog clock: twelve markers, hour and minute hands.
    fn render_analog(&self, app: &App) {
        let Some(w) = self.get(Kind::AnalogClock) else { return };
        let now = clock::now();
        let mut m = Matrix::new(23, 23);
        let c = 11.0f32;
        for i in 0..12 {
            let a = i as f32 / 12.0 * std::f32::consts::TAU;
            let main = i % 3 == 0;
            m.set(
                (c + a.sin() * 10.0).round() as i32,
                (c - a.cos() * 10.0).round() as i32,
                if main { 1.0 } else { 0.45 },
            );
        }
        let hand = |m: &mut Matrix, turns: f32, len: f32, v: f32| {
            let a = turns * std::f32::consts::TAU;
            let mut r = 0.0;
            while r <= len {
                m.set((c + a.sin() * r).round() as i32, (c - a.cos() * r).round() as i32, v);
                r += 0.5;
            }
        };
        let minutes = now.minute as f32 / 60.0;
        hand(&mut m, (now.hour % 12) as f32 / 12.0 + minutes / 12.0, 5.5, 1.0);
        hand(&mut m, minutes, 8.5, 0.85);
        m.set(11, 11, 1.0);
        let (iw, ih, px) = raster::render(&m, &Self::style(app, 7));
        w.ui.set_matrix(ui::image_from_rgba(iw, ih, &px));
        w.ui.set_subcaption(format!("{:02}:{:02}", now.hour, now.minute).into());
    }

    pub fn set_media(&self, media: Option<&NowPlaying>) {
        let Some(w) = self.get(Kind::NowPlaying) else { return };
        w.ui.set_np_has(media.is_some());
        if let Some(m) = media {
            w.ui.set_np_playing(m.playing);
            w.ui.set_np_track(m.title.as_str().into());
            w.ui.set_np_artist(m.artist.as_str().into());
            match &m.artwork {
                Some(a) => {
                    w.ui.set_np_artwork(ui::image_from_rgba(a.width, a.height, &a.rgba));
                    w.ui.set_np_has_artwork(true);
                }
                None => w.ui.set_np_has_artwork(false),
            }
        } else {
            w.ui.set_np_playing(false);
            w.ui.set_np_artist("".into());
            w.ui.set_np_has_artwork(false);
        }
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

fn sky_index(code: u32) -> i32 {
    match Sky::from_code(code) {
        Sky::Clear => 0,
        Sky::PartlyCloudy => 1,
        Sky::Cloudy => 2,
        Sky::Fog => 3,
        Sky::Rain => 4,
        Sky::Snow => 5,
        Sky::Storm => 6,
    }
}

fn hour_label(hour: u32) -> String {
    let h = match hour % 12 {
        0 => 12,
        h => h,
    };
    format!("{h} {}", if hour < 12 { "AM" } else { "PM" })
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

fn days_in_month(year: u16, month: u16) -> u16 {
    match month {
        2 if (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// 6×7 Monday-first month grid: `(day, is_today, outside_month)`.
/// `weekday` is today's day of week with Sunday = 0 (as `GetLocalTime`).
fn month_grid(year: u16, month: u16, day: u16, weekday: u16) -> Vec<(u16, bool, bool)> {
    let first_sunday_based = (weekday + 7 - (day - 1) % 7) % 7;
    let lead = (first_sunday_based + 6) % 7;
    let dim = days_in_month(year, month);
    let prev_dim = if month == 1 { 31 } else { days_in_month(year, month - 1) };
    (0..42i32)
        .map(|i| {
            let d = i - lead as i32 + 1;
            if d < 1 {
                ((prev_dim as i32 + d) as u16, false, true)
            } else if d > dim as i32 {
                ((d - dim as i32) as u16, false, true)
            } else {
                (d as u16, d as u16 == day, false)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn october_2026_starts_on_thursday() {
        // Thu 29 Oct 2026; 1 Oct 2026 is a Thursday (Monday-first index 3).
        let g = month_grid(2026, 10, 29, 4);
        assert_eq!(g[3], (1, false, false));
        assert_eq!(g[2], (30, false, true));
        assert_eq!(g[31], (29, true, false));
        assert_eq!(g[34], (1, false, true));
    }

    #[test]
    fn leap_february() {
        assert_eq!(days_in_month(2028, 2), 29);
        assert_eq!(days_in_month(2026, 2), 28);
    }
}
