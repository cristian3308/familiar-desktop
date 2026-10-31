//! Cursor effects: click ripples and the shake-to-find halo, drawn in a
//! small click-through window that follows the event position.

use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use familiar_sys::ghost::Ghost;
use familiar_sys::mouse::MouseFx;
use familiar_sys::window::{self, Rect, SurfaceOptions};
use familiar_sys::{Hwnd, windows_list};
use slint::{ComponentHandle, Timer, TimerMode};

use crate::app::{App, with_app};
use crate::ui::{self, FxWindow};

pub struct CursorFx {
    pub ui: FxWindow,
    hwnd: Cell<Option<Hwnd>>,
    arm: Timer,
    done: Timer,
}

impl CursorFx {
    pub fn new() -> Result<Self, slint::PlatformError> {
        Ok(Self { ui: FxWindow::new()?, hwnd: Cell::new(None), arm: Timer::default(), done: Timer::default() })
    }

    pub fn play(&self, app: &App, fx: MouseFx) {
        let (x, y, size, halo, ms) = match fx {
            MouseFx::Click { x, y } => (x, y, 64.0, false, 420),
            MouseFx::Shake { x, y } => (x, y, 220.0, true, 680),
        };
        let px = (size * app.monitor.get().scale) as i32;
        let rect = Rect { x: x - px / 2, y: y - px / 2, w: px, h: px };
        match self.hwnd.get() {
            None => {
                let win = self.ui.window();
                win.set_position(slint::PhysicalPosition::new(rect.x, rect.y));
                win.set_size(slint::PhysicalSize::new(rect.w as u32, rect.h as u32));
                if self.ui.show().is_err() {
                    return;
                }
                let Some(h) = ui::hwnd_of(win) else { return };
                window::apply_surface(
                    h.get(),
                    SurfaceOptions { tool_window: true, no_activate: true, topmost: true, click_through: true },
                );
                familiar_sys::dwm::disable_transitions(h.get());
                self.hwnd.set(Some(h));
            }
            Some(h) => {
                ui::set_bounds(self.ui.window(), rect);
                window::show_no_activate(h.get(), true);
                window::raise_topmost(h.get());
            }
        }
        // Reset to the start state, then flip on the next frame so the
        // Slint animations run from scratch.
        self.ui.set_halo(halo);
        self.ui.set_active(false);
        self.arm.start(TimerMode::SingleShot, Duration::from_millis(16), || {
            with_app(|a| {
                if let Some(fx) = a.fx.borrow().as_ref() {
                    fx.ui.set_active(true);
                }
            })
        });
        self.done.start(TimerMode::SingleShot, Duration::from_millis(ms), || {
            with_app(|a| {
                if let Some(fx) = a.fx.borrow().as_ref()
                    && let Some(h) = fx.hwnd.get()
                {
                    window::show_no_activate(h.get(), false);
                }
            })
        });
    }

    pub fn apply_theme(&self, app: &App) {
        ui::push_theme(&self.ui, &app.theme.borrow());
    }
}

#[derive(Clone, Copy)]
enum Ease {
    /// Decelerate: fast start, soft landing (opening).
    Out,
    /// Soft start and soft end (minimising, restoring).
    InOut,
    /// Constant speed: the genie shapes its own phases.
    Linear,
}

#[derive(Clone, Copy)]
struct Flight {
    from: Rect,
    to: Rect,
    start: Instant,
    ms: f32,
    ease: Ease,
    /// Progress until which the ghost fades in (0.0 = opaque from the start).
    fade_in: f32,
    /// Progress after which the ghost fades out (1.0 = never).
    fade_out: f32,
    /// Hide the real window during the flight (it is on screen already).
    hide: bool,
    /// macOS genie: the window bends into `to` (the dock icon) strip by
    /// strip. `reverse` plays it backwards (restoring from the dock).
    genie: Option<bool>,
}

impl Flight {
    fn opacity(&self, t: f32) -> u8 {
        let mut a = 1.0;
        if self.fade_in > 0.0 && t < self.fade_in {
            a = smooth(t / self.fade_in);
        }
        if t > self.fade_out {
            a *= 1.0 - smooth((t - self.fade_out) / (1.0 - self.fade_out));
        }
        (a * 255.0).round() as u8
    }
}

/// Thumbnails used for the genie effect: thin strips keep the curved
/// sides smooth (240 measured fine on a 1080p screen; 64 looked stepped).
const GENIE_STRIPS: usize = 240;

/// Strip count, overridable with FAMILIAR_GENIE_STRIPS for tuning.
fn genie_strips() -> usize {
    std::env::var("FAMILIAR_GENIE_STRIPS").ok().and_then(|v| v.parse().ok()).unwrap_or(GENIE_STRIPS).clamp(8, 400)
}

fn smooth(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

fn ease(e: Ease, t: f32) -> f32 {
    match e {
        // Quintic out: quick, then a long gentle settle (macOS feel).
        Ease::Out => 1.0 - (1.0 - t).powi(5),
        Ease::Linear => t,
        // Cubic in-out.
        Ease::InOut => {
            if t < 0.5 {
                4.0 * t * t * t
            } else {
                1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
            }
        }
    }
}

/// `icon` shrunk to the window's aspect ratio, so the ghost scales evenly
/// instead of being squashed into a square.
fn fit(icon: Rect, window: Rect) -> Rect {
    let aspect = window.w as f32 / window.h.max(1) as f32;
    let side = icon.w.max(icon.h) as f32;
    let (w, h) = if aspect >= 1.0 { (side, side / aspect) } else { (side * aspect, side) };
    let (cx, cy) = (icon.x as f32 + icon.w as f32 / 2.0, icon.y as f32 + icon.h as f32 / 2.0);
    Rect {
        x: (cx - w / 2.0).round() as i32,
        y: (cy - h / 2.0).round() as i32,
        w: w.round() as i32,
        h: h.round() as i32,
    }
}

/// Strip `i` of `n` at genie progress `t` (0 = window, 1 = inside the icon).
///
/// Like macOS: first the window's bottom edge stretches down into the icon
/// while its sides curve towards it (an S-curve that is untouched at the
/// top and icon-wide at the bottom); then the whole body pours down through
/// that funnel. The squeeze depends on the strip's position on screen, so
/// content narrows as it slides down.
fn genie_strip(w: Rect, icon: Rect, i: usize, n: usize, t: f32) -> Rect {
    let s0 = i as f32 / n as f32;
    let s1 = (i + 1) as f32 / n as f32;
    let lerp = |a: f32, b: f32, k: f32| a + (b - a) * k;
    // Phase 1 (0..0.55): stretch and bend, easing out so it starts at
    // once. Phase 2 (0.2..1): slide in, easing in and out. They overlap,
    // so the motion never stops between phases.
    let bend = {
        let v = (t / 0.55).clamp(0.0, 1.0);
        1.0 - (1.0 - v) * (1.0 - v)
    };
    let slide = smooth((t - 0.2) / 0.8);
    let top = lerp(w.y as f32, icon.y as f32, slide);
    let bottom = lerp((w.y + w.h) as f32, (icon.y + icon.h) as f32, bend);
    let y = |s: f32| lerp(top, bottom, s);
    let (y0, y1) = (y(s0), y(s1));
    // Where this strip is between the window's top and the icon's bottom.
    let span = ((icon.y + icon.h) - w.y).max(1) as f32;
    let f = (((y0 + y1) / 2.0 - w.y as f32) / span).clamp(0.0, 1.0);
    let k = (bend * smooth(f)).max(slide * slide);
    let left = lerp(w.x as f32, icon.x as f32, k);
    let right = lerp((w.x + w.w) as f32, (icon.x + icon.w) as f32, k);
    Rect {
        x: left.round() as i32,
        y: y0.floor() as i32,
        w: (right.round() - left.round()).max(1.0) as i32,
        // Overlap by a pixel so neighbouring strips never leave a seam.
        h: (y1.ceil() - y0.floor()).max(1.0) as i32 + 1,
    }
}

/// macOS-style window animations: a live thumbnail of the window zooms out
/// of its dock icon when it opens or is restored, and shrinks into it when
/// it is minimised. DWM composites the thumbnail, so frames cost almost
/// nothing, and each frame waits for v-sync so motion is even.
pub struct WindowFx {
    ghost: RefCell<Option<Ghost>>,
    /// Last animated window: Windows often reports a new window as shown
    /// twice (show + uncloak), which would restart the flight mid-way.
    last: Cell<Option<(Hwnd, Instant)>>,
    flight: Cell<Option<Flight>>,
    timer: Timer,
}

impl WindowFx {
    pub fn new() -> Self {
        Self { ghost: RefCell::new(None), last: Cell::new(None), flight: Cell::new(None), timer: Timer::default() }
    }

    /// The window just opened.
    pub fn open(&self, app: &App, hwnd: Hwnd) {
        let Some(to) = windows_list::window_rect(hwnd) else { return };
        if to.w < 120 || to.h < 80 {
            return;
        }
        // From its dock icon, or a gentle zoom from 92 % when it has none.
        let (from, fade_in) = match app.dock.as_ref().and_then(|d| d.icon_rect_for(app, hwnd)) {
            Some(icon) => (fit(icon, to), 0.18),
            None => (shrink(to, 0.92), 0.6),
        };
        let f = Flight {
            from,
            to,
            start: Instant::now(),
            ms: 340.0,
            ease: Ease::Out,
            fade_in,
            fade_out: 1.0,
            hide: true,
            genie: None,
        };
        self.fly(app, hwnd, f);
    }

    /// The window was restored from the dock.
    pub fn restore(&self, app: &App, hwnd: Hwnd) {
        let Some(win) = windows_list::window_rect(hwnd) else { return };
        let Some(icon) = app.dock.as_ref().and_then(|d| d.icon_rect_for(app, hwnd)) else {
            return self.open(app, hwnd);
        };
        if win.w < 120 || win.h < 80 {
            return;
        }
        let genie = app.cfg.borrow().genie_effect;
        let f = if genie {
            Flight {
                from: win,
                to: icon,
                start: Instant::now(),
                ms: 540.0,
                ease: Ease::Linear,
                fade_in: 0.0,
                fade_out: 1.0,
                hide: true,
                genie: Some(true),
            }
        } else {
            Flight {
                from: fit(icon, win),
                to: win,
                start: Instant::now(),
                ms: 320.0,
                ease: Ease::Out,
                fade_in: 0.2,
                fade_out: 1.0,
                hide: true,
                genie: None,
            }
        };
        self.fly(app, hwnd, f);
    }

    /// The window started minimising.
    pub fn minimize(&self, app: &App, hwnd: Hwnd) {
        // With Windows' own animations off the window is already iconic
        // here; start from where it was restored.
        let Some(from) = windows_list::restored_rect(hwnd) else { return };
        if from.w < 120 || from.h < 80 {
            return;
        }
        let icon = app.dock.as_ref().and_then(|d| d.icon_rect_for(app, hwnd)).unwrap_or_else(|| {
            let m = app.monitor.get().bounds;
            Rect { x: m.x + m.w / 2 - 24, y: m.y + m.h - 60, w: 48, h: 48 }
        });
        let genie = app.cfg.borrow().genie_effect;
        let f = Flight {
            from,
            to: if genie { icon } else { fit(icon, from) },
            start: Instant::now(),
            ms: if genie { 600.0 } else { 330.0 },
            ease: if genie { Ease::Linear } else { Ease::InOut },
            fade_in: 0.0,
            fade_out: if genie { 0.9 } else { 0.7 },
            hide: false,
            genie: genie.then_some(false),
        };
        self.fly(app, hwnd, f);
    }

    fn fly(&self, app: &App, hwnd: Hwnd, f: Flight) {
        if f.hide {
            if self.last.get().is_some_and(|(h, at)| h == hwnd && at.elapsed() < Duration::from_millis(600)) {
                log::debug!("window fx: duplicate show of {hwnd:?}");
                return;
            }
            self.last.set(Some((hwnd, Instant::now())));
        }
        if self.ghost.borrow().is_none() {
            *self.ghost.borrow_mut() = Ghost::new();
        }
        let area = app.monitor.get().bounds;
        let strips = if f.genie.is_some() { genie_strips() } else { 1 };
        let started = self.ghost.borrow_mut().as_mut().is_some_and(|g| g.begin(hwnd, area, f.hide, strips));
        if !started {
            log::debug!("window fx: no thumbnail for {hwnd:?}");
            return;
        }
        let thumbs = self.ghost.borrow().as_ref().map_or(0, |g| g.strips());
        log::debug!("window fx: {:?} -> {:?} genie={:?} thumbs={thumbs}", f.from, f.to, f.genie);
        // Clock starts once the ghost exists, so setup time never eats
        // into the first frames.
        self.flight.set(Some(Flight { start: Instant::now(), ..f }));
        self.step();
        self.timer.start(TimerMode::Repeated, Duration::from_millis(1), || {
            with_app(|a| a.window_fx.step());
        });
    }

    fn step(&self) {
        let Some(f) = self.flight.get() else { return };
        let t = (f.start.elapsed().as_secs_f32() * 1000.0 / f.ms).min(1.0);
        if t >= 1.0 {
            self.flight.set(None);
            self.timer.stop();
            if let Some(g) = self.ghost.borrow_mut().as_mut() {
                g.end();
            }
            return;
        }
        let opacity = f.opacity(t);
        if let Some(g) = self.ghost.borrow().as_ref() {
            if let Some(reverse) = f.genie {
                let p = ease(f.ease, if reverse { 1.0 - t } else { t });
                let n = g.strips();
                for i in 0..n {
                    g.frame_strip(i, genie_strip(f.from, f.to, i, n, p), opacity);
                }
            } else {
                let e = ease(f.ease, t);
                let lerp = |a: i32, b: i32| a as f32 + (b - a) as f32 * e;
                let (x, y) = (lerp(f.from.x, f.to.x), lerp(f.from.y, f.to.y));
                let (r, b) = (lerp(f.from.x + f.from.w, f.to.x + f.to.w), lerp(f.from.y + f.from.h, f.to.y + f.to.h));
                let rect = Rect {
                    x: x.round() as i32,
                    y: y.round() as i32,
                    w: (r - x).round() as i32,
                    h: (b - y).round() as i32,
                };
                g.frame(rect, opacity);
            }
        }
        // Pace on the compositor: one update per displayed frame.
        Ghost::vsync();
    }
}

impl Drop for WindowFx {
    fn drop(&mut self) {
        // Never leave a window transparent.
        if let Some(g) = self.ghost.borrow_mut().as_mut() {
            g.end();
        }
    }
}

fn shrink(r: Rect, k: f32) -> Rect {
    let (w, h) = ((r.w as f32 * k) as i32, (r.h as f32 * k) as i32);
    Rect { x: r.x + (r.w - w) / 2, y: r.y + (r.h - h) / 2, w, h }
}

#[cfg(test)]
mod tests {
    use super::{Rect, genie_strip};

    const WIN: Rect = Rect { x: 300, y: 160, w: 1100, h: 700 };
    const ICON: Rect = Rect { x: 1095, y: 1018, w: 45, h: 45 };

    #[test]
    fn genie_starts_as_the_window_and_ends_in_the_icon() {
        let n = 100;
        let first = genie_strip(WIN, ICON, 0, n, 0.0);
        assert_eq!((first.x, first.y, first.w), (WIN.x, WIN.y, WIN.w));
        for i in 0..n {
            let r = genie_strip(WIN, ICON, i, n, 1.0);
            assert!(r.x >= ICON.x - 1 && r.x + r.w <= ICON.x + ICON.w + 1, "strip {i}: {r:?}");
            assert!(r.y >= ICON.y - 1 && r.y <= ICON.y + ICON.h, "strip {i}: {r:?}");
        }
    }

    #[test]
    fn genie_sides_curve_smoothly() {
        // Halfway through, the top is still wide, the bottom near the icon,
        // and widths never jump between neighbouring strips.
        let n = 240;
        let rows: Vec<Rect> = (0..n).map(|i| genie_strip(WIN, ICON, i, n, 0.5)).collect();
        assert!(rows[0].w > WIN.w / 2);
        assert!(rows[n - 1].w < WIN.w / 4);
        for pair in rows.windows(2) {
            assert!((pair[0].w - pair[1].w).abs() <= 24, "{:?} -> {:?}", pair[0], pair[1]);
            assert!(pair[1].w <= pair[0].w + 1, "widens downwards: {:?} -> {:?}", pair[0], pair[1]);
        }
    }
}
