//! Cursor effects: click ripples and the shake-to-find halo, drawn in a
//! small click-through window that follows the event position.

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use familiar_sys::canvas::{self, Canvas};
use familiar_sys::ghost::{Frames, Ghost};
use familiar_sys::mouse::MouseFx;
use familiar_sys::window::{self, Rect, SurfaceOptions};
use familiar_sys::{Hwnd, windows_list};
use slint::{ComponentHandle, Timer, TimerMode};

use crate::app::{App, post, with_app};
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

/// Genie geometry at progress `t` (0 = window, 1 = inside the icon).
///
/// Like macOS: first the window's bottom edge stretches down into the icon
/// while its sides curve towards it (an S-curve that is untouched at the
/// top and icon-wide at the bottom); then the whole body pours down through
/// that funnel. The squeeze depends on a row's position on screen, so
/// content narrows as it slides down.
#[derive(Clone, Copy, Debug)]
struct Genie {
    win: Rect,
    icon: Rect,
    /// Screen rows the window currently spans.
    top: f32,
    bottom: f32,
    bend: f32,
    slide: f32,
}

impl Genie {
    fn at(win: Rect, icon: Rect, t: f32) -> Self {
        // Phase 1 (0..0.55): stretch and bend, easing out so it starts at
        // once. Phase 2 (0.2..1): slide in, easing in and out. They overlap,
        // so the motion never stops between phases.
        let bend = {
            let v = (t / 0.55).clamp(0.0, 1.0);
            1.0 - (1.0 - v) * (1.0 - v)
        };
        let slide = smooth((t - 0.2) / 0.8);
        let lerp = |a: f32, b: f32, k: f32| a + (b - a) * k;
        Self {
            win,
            icon,
            top: lerp(win.y as f32, icon.y as f32, slide),
            bottom: lerp((win.y + win.h) as f32, (icon.y + icon.h) as f32, bend),
            bend,
            slide,
        }
    }

    /// Left and right edge (screen x) of the row at screen `y`.
    fn edges(&self, y: f32) -> (f32, f32) {
        let span = ((self.icon.y + self.icon.h) - self.win.y).max(1) as f32;
        let f = ((y - self.win.y as f32) / span).clamp(0.0, 1.0);
        let k = (self.bend * smooth(f)).max(self.slide * self.slide);
        let lerp = |a: f32, b: f32| a + (b - a) * k;
        (
            lerp(self.win.x as f32, self.icon.x as f32),
            lerp((self.win.x + self.win.w) as f32, (self.icon.x + self.icon.w) as f32),
        )
    }

    /// Source row fraction (0..1) shown at screen `y`, if any.
    fn source_row(&self, y: f32) -> Option<f32> {
        let s = (y - self.top) / (self.bottom - self.top).max(0.001);
        (0.0..1.0).contains(&s).then_some(s)
    }
}

/// Strip `i` of `n` at genie progress `t`, as a rectangle.
#[cfg(test)]
fn genie_strip(w: Rect, icon: Rect, i: usize, n: usize, t: f32) -> Rect {
    let g = Genie::at(w, icon, t);
    let y = |s: f32| g.top + (g.bottom - g.top) * s;
    let (y0, y1) = (y(i as f32 / n as f32), y((i + 1) as f32 / n as f32));
    let (left, right) = g.edges((y0 + y1) / 2.0);
    Rect {
        x: left.round() as i32,
        y: y0.floor() as i32,
        w: (right.round() - left.round()).max(1.0) as i32,
        h: (y1.ceil() - y0.floor()).max(1.0) as i32 + 1,
    }
}

/// Renders the genie frame into `out` (BGRA premultiplied, `bbox` sized):
/// every screen row samples its source row and stretches it between the
/// curved edges, with anti-aliased sides.
fn render_genie(src: &[u8], sw: usize, sh: usize, g: &Genie, bbox: Rect, opacity: f32, out: &mut [u8]) {
    out.fill(0);
    let (bw, bh) = (bbox.w.max(0) as usize, bbox.h.max(0) as usize);
    if sw == 0 || sh == 0 || out.len() < bw * bh * 4 {
        return;
    }
    let y_from = (g.top.floor() as i32 - bbox.y).max(0) as usize;
    let y_to = ((g.bottom.ceil() as i32 - bbox.y).max(0) as usize).min(bh);
    for oy in y_from..y_to {
        let y = (oy as i32 + bbox.y) as f32 + 0.5;
        let Some(s) = g.source_row(y) else { continue };
        let sy = ((s * sh as f32) as usize).min(sh - 1);
        let row = &src[sy * sw * 4..(sy + 1) * sw * 4];
        let (left, right) = g.edges(y);
        let width = (right - left).max(0.001);
        let x_from = (left.floor() as i32 - bbox.x).max(0) as usize;
        let x_to = ((right.ceil() as i32 - bbox.x).max(0) as usize).min(bw);
        let line = &mut out[oy * bw * 4..(oy + 1) * bw * 4];
        for ox in x_from..x_to {
            let x = (ox as i32 + bbox.x) as f32;
            // Coverage of this pixel by [left, right): soft edges.
            let cover = ((x + 1.0).min(right) - x.max(left)).clamp(0.0, 1.0) * opacity;
            if cover <= 0.0 {
                continue;
            }
            let u = ((x + 0.5 - left) / width).clamp(0.0, 0.9999);
            let sx = (u * sw as f32) as usize;
            let p = &row[sx * 4..sx * 4 + 4];
            let d = &mut line[ox * 4..ox * 4 + 4];
            let a = cover * f32::from(p[3]) / 255.0;
            d[0] = (f32::from(p[0]) * a) as u8;
            d[1] = (f32::from(p[1]) * a) as u8;
            d[2] = (f32::from(p[2]) * a) as u8;
            d[3] = (255.0 * a) as u8;
        }
    }
}

fn union(a: Rect, b: Rect) -> Rect {
    let (x0, y0) = (a.x.min(b.x), a.y.min(b.y));
    let (x1, y1) = ((a.x + a.w).max(b.x + b.w), (a.y + a.h).max(b.y + b.h));
    Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
}

fn clip(r: Rect, to: Rect) -> Rect {
    let (x0, y0) = (r.x.max(to.x), r.y.max(to.y));
    let (x1, y1) = ((r.x + r.w).min(to.x + to.w), (r.y + r.h).min(to.y + to.h));
    Rect { x: x0, y: y0, w: (x1 - x0).max(0), h: (y1 - y0).max(0) }
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
    /// Bumped for every flight; an animation thread stops as soon as it
    /// no longer matches (a newer flight took over the ghost).
    generation: Arc<AtomicU64>,
    /// Layered window the genie is drawn into, created on first use.
    canvas: RefCell<Option<Arc<Mutex<Canvas>>>>,
}

impl WindowFx {
    pub fn new() -> Self {
        Self {
            ghost: RefCell::new(None),
            last: Cell::new(None),
            generation: Arc::new(AtomicU64::new(0)),
            canvas: RefCell::new(None),
        }
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
        if f.genie.is_some() {
            return self.fly_genie(hwnd, area, f);
        }
        let strips = 1;
        // The real window's on-screen size, to crop DWM's invisible borders.
        let window = if f.genie.is_some() || !f.hide { f.from } else { f.to };
        let started =
            self.ghost.borrow_mut().as_mut().is_some_and(|g| g.begin(hwnd, area, (window.w, window.h), f.hide, strips));
        if !started {
            log::debug!("window fx: no thumbnail for {hwnd:?}");
            return;
        }
        let Some(frames) = self.ghost.borrow().as_ref().map(Ghost::frames) else { return };
        log::debug!("window fx: {:?} -> {:?} genie={:?} thumbs={}", f.from, f.to, f.genie, frames.strips());
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        // First frame right away: the real window is already transparent.
        draw(&f, &frames, 0.0);
        let current = self.generation.clone();
        // Frames come from their own thread, paced by DWM, so the dock,
        // island and widgets repainting on the UI thread never make the
        // animation skip a beat.
        let flight = Flight { start: Instant::now(), ..f };
        let spawned = std::thread::Builder::new().name("window-fx".into()).spawn(move || {
            let mut count = 0u32;
            loop {
                if current.load(Ordering::SeqCst) != generation {
                    return;
                }
                let t = (flight.start.elapsed().as_secs_f32() * 1000.0 / flight.ms).min(1.0);
                draw(&flight, &frames, t);
                count += 1;
                if t >= 1.0 {
                    let fps = count as f32 / flight.start.elapsed().as_secs_f32();
                    log::debug!("window fx: {count} frames, {fps:.0} fps");
                    post(move |a| a.window_fx.finish(generation));
                    return;
                }
                Ghost::vsync();
            }
        });
        if spawned.is_err() {
            self.finish(generation);
        }
    }

    /// Genie: one snapshot of the window, bent on the CPU into a layered
    /// window with one `UpdateLayeredWindow` per frame. (Hundreds of DWM
    /// thumbnail strips managed only 12-30 fps.)
    fn fly_genie(&self, hwnd: Hwnd, area: Rect, f: Flight) {
        let reverse = f.genie == Some(true);
        let win = f.from;
        // Snapshot through a DWM thumbnail placed where the window is: it
        // works for minimised windows too, and while the real one is hidden.
        let started = self.ghost.borrow_mut().as_mut().is_some_and(|g| g.begin(hwnd, area, (win.w, win.h), f.hide, 1));
        if !started {
            log::debug!("window fx: no thumbnail for {hwnd:?}");
            return;
        }
        let shot = clip(win, area);
        if let Some(frames) = self.ghost.borrow().as_ref().map(Ghost::frames) {
            frames.frame(win, 255);
        }
        Ghost::vsync();
        Ghost::vsync();
        let pixels = canvas::capture_screen(shot);
        let Some(pixels) = pixels else {
            if let Some(g) = self.ghost.borrow_mut().as_mut() {
                g.end();
            }
            return;
        };
        if self.canvas.borrow().is_none() {
            *self.canvas.borrow_mut() = Canvas::new().map(|c| Arc::new(Mutex::new(c)));
        }
        let Some(canvas) = self.canvas.borrow().clone() else { return };
        let bbox = clip(union(shot, f.to), area);
        let (sw, sh) = (shot.w.max(1) as usize, shot.h.max(1) as usize);
        let genie_at = move |t: f32| {
            let p = ease(f.ease, if reverse { 1.0 - t } else { t });
            Genie::at(shot, f.to, p)
        };
        {
            let mut c = canvas.lock().unwrap_or_else(|e| e.into_inner());
            if !c.begin(bbox) {
                return;
            }
            c.present(|out| render_genie(&pixels, sw, sh, &genie_at(0.0), bbox, f.opacity(0.0) as f32 / 255.0, out));
            c.show();
        }
        // The canvas now shows the window: the thumbnail can go.
        if let Some(g) = self.ghost.borrow_mut().as_mut() {
            g.drop_thumbnails();
        }
        log::debug!("window fx: genie {:?} -> {:?} reverse={reverse}", shot, f.to);
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let current = self.generation.clone();
        let flight = Flight { start: Instant::now(), ..f };
        let spawned = std::thread::Builder::new().name("genie".into()).spawn(move || {
            let mut count = 0u32;
            loop {
                if current.load(Ordering::SeqCst) != generation {
                    return;
                }
                let t = (flight.start.elapsed().as_secs_f32() * 1000.0 / flight.ms).min(1.0);
                let g = genie_at(t);
                let opacity = flight.opacity(t) as f32 / 255.0;
                canvas
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .present(|out| render_genie(&pixels, sw, sh, &g, bbox, opacity, out));
                count += 1;
                if t >= 1.0 {
                    let fps = count as f32 / flight.start.elapsed().as_secs_f32();
                    log::debug!("window fx: genie {count} frames, {fps:.0} fps");
                    post(move |a| a.window_fx.finish(generation));
                    return;
                }
                Ghost::vsync();
            }
        });
        if spawned.is_err() {
            self.finish(generation);
        }
    }

    /// The flight's thread drew its last frame: hand back the real window.
    fn finish(&self, generation: u64) {
        if self.generation.load(Ordering::SeqCst) != generation {
            return;
        }
        if let Some(c) = self.canvas.borrow().as_ref() {
            c.lock().unwrap_or_else(|e| e.into_inner()).hide();
        }
        if let Some(g) = self.ghost.borrow_mut().as_mut() {
            g.end();
        }
    }
}

/// Places the ghost (a plain zoom) for progress `t` (0..=1) of `f`.
fn draw(f: &Flight, frames: &Frames, t: f32) {
    let opacity = f.opacity(t);
    let e = ease(f.ease, t);
    let lerp = |a: i32, b: i32| a as f32 + (b - a) as f32 * e;
    let (x, y) = (lerp(f.from.x, f.to.x), lerp(f.from.y, f.to.y));
    let (r, b) = (lerp(f.from.x + f.from.w, f.to.x + f.to.w), lerp(f.from.y + f.from.h, f.to.y + f.to.h));
    let rect = Rect { x: x.round() as i32, y: y.round() as i32, w: (r - x).round() as i32, h: (b - y).round() as i32 };
    frames.frame(rect, opacity);
}

impl Drop for WindowFx {
    fn drop(&mut self) {
        // Stop any animation thread, and never leave a window transparent.
        self.generation.fetch_add(1, Ordering::SeqCst);
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

    #[test]
    fn genie_frame_starts_as_the_window_and_ends_in_the_icon() {
        use super::{Genie, render_genie};
        let (sw, sh) = (40usize, 30usize);
        let src = vec![200u8; sw * sh * 4];
        let win = Rect { x: 0, y: 0, w: 40, h: 30 };
        let icon = Rect { x: 30, y: 60, w: 6, h: 6 };
        let bbox = Rect { x: 0, y: 0, w: 40, h: 66 };
        let mut out = vec![0u8; 40 * 66 * 4];
        render_genie(&src, sw, sh, &Genie::at(win, icon, 0.0), bbox, 1.0, &mut out);
        let lit = |out: &[u8]| out.chunks(4).filter(|p| p[3] > 0).count();
        assert_eq!(lit(&out), 40 * 30, "t=0 covers exactly the window");
        render_genie(&src, sw, sh, &Genie::at(win, icon, 1.0), bbox, 1.0, &mut out);
        for (i, p) in out.chunks(4).enumerate() {
            if p[3] > 0 {
                let (x, y) = ((i % 40) as i32, (i / 40) as i32);
                assert!((29..=36).contains(&x) && (59..=66).contains(&y), "pixel {x},{y} outside the icon");
            }
        }
    }
}
