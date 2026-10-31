//! Alt+W: switch to the next image of the wallpaper folder with a
//! transition (fade, circle, wipe or dots).
//!
//! The new image is decoded on a worker thread at the monitor's size, drawn
//! over the desktop by a click-through window at the bottom of the z-order,
//! revealed, and only then set as the Windows wallpaper; the overlay hides
//! once Explorer has repainted underneath and its pixels are dropped.

use std::cell::Cell;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use familiar_sys::Hwnd;
use familiar_sys::wallpaper;
use familiar_sys::window::{self, SurfaceOptions};
use slint::{ComponentHandle, Timer, TimerMode};

use crate::app::{App, post, with_app};
use crate::ui::{self, WallpaperWindow};

const REVEAL: Duration = Duration::from_millis(900);
/// Time for Explorer to repaint the desktop before the overlay goes.
const SETTLE: Duration = Duration::from_millis(450);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    Fade,
    Circle,
    Wipe,
    Dots,
}

impl Transition {
    const ALL: [Transition; 4] = [Transition::Fade, Transition::Circle, Transition::Wipe, Transition::Dots];

    pub fn id(self) -> &'static str {
        match self {
            Transition::Fade => "fade",
            Transition::Circle => "circle",
            Transition::Wipe => "wipe",
            Transition::Dots => "dots",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Transition::Fade => "Fade",
            Transition::Circle => "Circle",
            Transition::Wipe => "Wipe",
            Transition::Dots => "Dots",
        }
    }

    pub fn from_id(id: &str) -> Self {
        Self::ALL.into_iter().find(|t| t.id().eq_ignore_ascii_case(id)).unwrap_or(Transition::Fade)
    }

    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|t| *t == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    fn ui_index(self) -> i32 {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0) as i32
    }
}

pub struct WallpaperFx {
    ui: WallpaperWindow,
    hwnd: Cell<Option<Hwnd>>,
    busy: Cell<bool>,
    started: Cell<Option<Instant>>,
    anim: Timer,
    settle: Timer,
}

impl WallpaperFx {
    pub fn new() -> Result<Self, slint::PlatformError> {
        Ok(Self {
            ui: WallpaperWindow::new()?,
            hwnd: Cell::new(None),
            busy: Cell::new(false),
            started: Cell::new(None),
            anim: Timer::default(),
            settle: Timer::default(),
        })
    }

    /// The folder Alt+W cycles through: the configured one, else the
    /// folder of the current wallpaper.
    fn folder(app: &App) -> Option<PathBuf> {
        let configured = app.cfg.borrow().wallpaper.folder.trim().to_string();
        if !configured.is_empty() {
            return Some(PathBuf::from(configured));
        }
        wallpaper::current().and_then(|p| p.parent().map(|d| d.to_path_buf()))
    }

    /// Alt+W.
    pub fn next(&self, app: &App) {
        if self.busy.get() {
            return;
        }
        let Some(dir) = Self::folder(app) else {
            log::info!("wallpaper: no folder to rotate through");
            return;
        };
        let list = wallpaper::images_in(&dir);
        let current = wallpaper::current();
        let Some(next) = wallpaper::next_after(&list, current.as_deref()).cloned() else {
            log::info!("wallpaper: no images in {}", dir.display());
            return;
        };
        if Some(&next) == current.as_ref() {
            return;
        }
        self.busy.set(true);
        let mon = app.monitor.get().bounds;
        let (w, h) = (mon.w.max(1) as u32, mon.h.max(1) as u32);
        let spawned = std::thread::Builder::new().name("wallpaper".into()).spawn(move || {
            let pic = wallpaper::load_fill(&next, w, h);
            post(move |a| a.wallpaper_fx.loaded(a, next, pic));
        });
        if spawned.is_err() {
            self.busy.set(false);
        }
    }

    fn loaded(&self, app: &App, path: PathBuf, pic: Option<wallpaper::Picture>) {
        let Some(pic) = pic else {
            log::warn!("wallpaper: cannot decode {}", path.display());
            self.busy.set(false);
            return;
        };
        let transition = Transition::from_id(&app.cfg.borrow().wallpaper.transition);
        let animate = app.cfg.borrow().window_animations && !app.is_calm();
        self.ui.set_picture(ui::image_from_rgba(pic.width, pic.height, &pic.rgba));
        drop(pic);
        self.ui.set_mode(transition.ui_index());
        self.ui.set_progress(0.0);
        if !animate {
            return self.apply(path);
        }
        let win = self.ui.window();
        ui::set_bounds(win, app.monitor.get().bounds);
        if self.ui.show().is_err() {
            return self.apply(path);
        }
        if self.hwnd.get().is_none()
            && let Some(h) = ui::hwnd_of(win)
        {
            window::apply_surface(
                h.get(),
                SurfaceOptions { tool_window: true, no_activate: true, topmost: false, click_through: true },
            );
            window::pin_to_bottom(h.get());
            self.hwnd.set(Some(h));
        }
        self.started.set(Some(Instant::now()));
        self.anim.start(TimerMode::Repeated, Duration::from_millis(16), move || {
            let path = path.clone();
            with_app(move |a| a.wallpaper_fx.tick(path))
        });
    }

    fn tick(&self, path: PathBuf) {
        let Some(t0) = self.started.get() else { return };
        let t = (t0.elapsed().as_secs_f32() / REVEAL.as_secs_f32()).min(1.0);
        // Ease in-out so the reveal starts and lands softly.
        let e = if t < 0.5 { 2.0 * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(2) / 2.0 };
        self.ui.set_progress(e);
        if t >= 1.0 {
            self.anim.stop();
            self.started.set(None);
            self.apply(path);
        }
    }

    /// Sets the Windows wallpaper, then drops the overlay once it is drawn.
    fn apply(&self, path: PathBuf) {
        if let Err(e) = wallpaper::set(&path) {
            log::warn!("wallpaper: setting {}: {e}", path.display());
        }
        self.settle.start(TimerMode::SingleShot, SETTLE, || {
            with_app(|a| {
                let fx = &a.wallpaper_fx;
                let _ = fx.ui.hide();
                fx.ui.set_picture(slint::Image::default());
                fx.busy.set(false);
            })
        });
    }
}

#[cfg(test)]
mod tests {
    use super::Transition;

    #[test]
    fn transitions_cycle() {
        let mut t = Transition::Fade;
        for _ in 0..4 {
            assert_eq!(Transition::from_id(t.id()), t);
            t = t.next();
        }
        assert_eq!(t, Transition::Fade);
        assert_eq!(Transition::from_id("?"), Transition::Fade);
    }
}
