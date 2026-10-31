//! Full-screen visualizer: after a few idle minutes with music playing the
//! screen fades to a large dot-matrix scene with the track's title. Any
//! keyboard or mouse input (seen through `GetLastInputInfo`, so the window
//! never needs focus) sends it away.

use std::cell::{Cell, RefCell};
use std::time::Duration;

use familiar_signal::Matrix;
use familiar_signal::raster::{self, DotStyle, Rgba};
use familiar_signal::scenes::{self, Scene, SceneState};
use familiar_sys::Hwnd;
use familiar_sys::window::{self, SurfaceOptions};
use slint::{ComponentHandle, Timer, TimerMode};

use crate::app::{App, with_app};
use crate::ui::{self, ScreensaverWindow};

const CHECK: Duration = Duration::from_secs(5);
const WATCH: Duration = Duration::from_millis(200);
/// Each scene plays this long before the next one.
const SCENE_TIME: u32 = 30 * 30;

pub struct Screensaver {
    ui: ScreensaverWindow,
    hwnd: Cell<Option<Hwnd>>,
    active: Cell<bool>,
    check: Timer,
    watch: Timer,
    /// Idle time when it appeared; less idle time later means input.
    idle_at_start: Cell<u32>,
    scene: Cell<usize>,
    frames: Cell<u32>,
    state: RefCell<SceneState>,
}

impl Screensaver {
    pub fn new() -> Result<Self, slint::PlatformError> {
        Ok(Self {
            ui: ScreensaverWindow::new()?,
            hwnd: Cell::new(None),
            active: Cell::new(false),
            check: Timer::default(),
            watch: Timer::default(),
            idle_at_start: Cell::new(0),
            scene: Cell::new(0),
            frames: Cell::new(0),
            state: RefCell::default(),
        })
    }

    pub fn start(&self) {
        self.check.start(TimerMode::Repeated, CHECK, || with_app(|a| a.screensaver.tick(a)));
    }

    pub fn is_active(&self) -> bool {
        self.active.get()
    }

    fn tick(&self, app: &App) {
        let cfg = app.cfg.borrow().screensaver.clone();
        if self.active.get() || !cfg.enabled {
            return;
        }
        let playing = app.media().is_some_and(|m| m.playing);
        let idle = familiar_sys::util::idle_ms();
        if playing && idle >= cfg.idle_minutes.max(1) * 60_000 && !app.is_calm() && !app.lock.is_visible() {
            self.show(app, idle);
        }
    }

    fn show(&self, app: &App, idle: u32) {
        ui::push_theme(&self.ui, &app.theme.borrow());
        if let Some(m) = app.media() {
            self.ui.set_track(m.title.as_str().into());
            self.ui.set_artist(m.artist.as_str().into());
        }
        self.ui.set_shown(false);
        let win = self.ui.window();
        ui::set_bounds(win, app.monitor.get().bounds);
        if self.ui.show().is_err() {
            return;
        }
        if self.hwnd.get().is_none()
            && let Some(h) = ui::hwnd_of(win)
        {
            window::apply_surface(
                h.get(),
                SurfaceOptions { tool_window: true, no_activate: true, topmost: true, click_through: false },
            );
            self.hwnd.set(Some(h));
        }
        if let Some(h) = self.hwnd.get() {
            window::raise_topmost(h.get());
        }
        self.active.set(true);
        self.idle_at_start.set(idle);
        self.frames.set(0);
        *self.state.borrow_mut() = SceneState::default();
        self.render(app, &[]);
        Timer::single_shot(Duration::from_millis(30), || with_app(|a| a.screensaver.ui.set_shown(true)));
        self.watch.start(TimerMode::Repeated, WATCH, || with_app(|a| a.screensaver.watch_input(a)));
        log::info!("screensaver on");
    }

    /// Any input since it appeared resets the idle counter below where it was.
    fn watch_input(&self, app: &App) {
        let idle = familiar_sys::util::idle_ms();
        let stopped = !app.media().is_some_and(|m| m.playing);
        if idle < self.idle_at_start.get() || stopped {
            self.hide();
        } else {
            self.idle_at_start.set(idle);
        }
    }

    pub fn hide(&self) {
        if !self.active.replace(false) {
            return;
        }
        self.watch.stop();
        let _ = self.ui.hide();
        // Let the next scene start fresh and free the frame.
        self.ui.set_matrix(slint::Image::default());
        self.scene.set((self.scene.get() + 1) % Scene::ALL.len());
        log::info!("screensaver off");
    }

    /// A spectrum frame (every other one is skipped: ~30 fps is plenty for
    /// a full-screen software render).
    pub fn render(&self, app: &App, bands: &[f32]) {
        if !self.active.get() {
            return;
        }
        let n = self.frames.get().wrapping_add(1);
        self.frames.set(n);
        if n % 2 == 1 && !bands.is_empty() {
            return;
        }
        if n.is_multiple_of(SCENE_TIME * 2) {
            self.scene.set((self.scene.get() + 1) % Scene::ALL.len());
            *self.state.borrow_mut() = SceneState::default();
        }
        let scene = Scene::ALL[self.scene.get() % Scene::ALL.len()];
        let mut m = Matrix::new(96, 40);
        scenes::draw(&mut m, scene, bands, &mut self.state.borrow_mut());
        let t = app.theme.borrow();
        let c = |c: familiar_theme::Color| Rgba(c.r, c.g, c.b, c.a);
        let style = DotStyle { pitch: 10, radius: 0.38, on: c(t.colors.accent), off: Rgba(255, 255, 255, 18) };
        drop(t);
        let (w, h, px) = raster::render(&m, &style);
        self.ui.set_matrix(ui::image_from_rgba(w, h, &px));
        self.ui.set_scene(format!("VISUALIZER · {}", scene.name()).into());
    }

    pub fn apply_theme(&self, app: &App) {
        ui::push_theme(&self.ui, &app.theme.borrow());
    }
}
