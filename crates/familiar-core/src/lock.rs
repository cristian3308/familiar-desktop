//! W8: splash and lock face, shown when Familiar starts and from the
//! Control Center. Visual only — Win+L is still the real (password) lock.

use std::cell::Cell;
use std::time::Duration;

use familiar_signal::raster::{self, DotStyle, Rgba};
use familiar_signal::{Matrix, font};
use familiar_sys::media::NowPlaying;
use familiar_sys::power::Battery;
use familiar_sys::window::{self, SurfaceOptions};
use familiar_sys::{Hwnd, clock, windows_list};
use slint::{ComponentHandle, Timer, TimerMode};

use crate::app::{App, with_app};
use crate::ui::{self, LockWindow};

const SPLASH: Duration = Duration::from_millis(1100);
const LEAVE: Duration = Duration::from_millis(400);

pub struct Lock {
    pub ui: LockWindow,
    hwnd: Cell<Option<Hwnd>>,
    visible: Cell<bool>,
    phase_timer: Timer,
    leave_timer: Timer,
}

impl Lock {
    pub fn new() -> Result<Self, slint::PlatformError> {
        let ui = LockWindow::new()?;
        ui.on_unlock(|| with_app(|a| a.lock.unlock()));
        Ok(Self {
            ui,
            hwnd: Cell::new(None),
            visible: Cell::new(false),
            phase_timer: Timer::default(),
            leave_timer: Timer::default(),
        })
    }

    pub fn is_visible(&self) -> bool {
        self.visible.get()
    }

    /// Lock face only.
    pub fn show(&self, app: &App) {
        self.present(app, false);
    }

    /// Splash (logo) first, then the lock face.
    pub fn show_with_splash(&self, app: &App) {
        self.present(app, true);
    }

    fn present(&self, app: &App, splash: bool) {
        ui::push_theme(&self.ui, &app.theme.borrow());
        self.load_backdrop();
        self.refresh_clock(app);
        self.ui.set_phase(if splash { 0 } else { 1 });
        self.ui.set_leaving(false);
        self.ui.set_appeared(false);
        let mon = app.monitor.get().bounds;
        let win = self.ui.window();
        win.set_position(slint::PhysicalPosition::new(mon.x, mon.y));
        win.set_size(slint::PhysicalSize::new(mon.w as u32, mon.h as u32));
        if let Err(e) = self.ui.show() {
            log::error!("lock: {e}");
            return;
        }
        self.visible.set(true);
        if let Some(hwnd) = ui::hwnd_of(win) {
            if self.hwnd.replace(Some(hwnd)).is_none() {
                window::apply_surface(
                    hwnd.get(),
                    SurfaceOptions { tool_window: true, no_activate: false, topmost: true, click_through: false },
                );
            }
            window::raise_topmost(hwnd.get());
            windows_list::activate(hwnd);
        }
        self.ui.invoke_focus_keys();
        Timer::single_shot(Duration::from_millis(16), || with_app(|a| a.lock.ui.set_appeared(true)));
        if splash {
            self.phase_timer.start(TimerMode::SingleShot, SPLASH, || with_app(|a| a.lock.ui.set_phase(1)));
        }
    }

    /// Decodes the wallpaper small on a worker thread, blurs it and shows
    /// it behind the lock face (plain black until it is ready).
    fn load_backdrop(&self) {
        let spawned = std::thread::Builder::new().name("lock-backdrop".into()).spawn(|| {
            let Some(path) = familiar_sys::wallpaper::current() else { return };
            let Some(mut pic) = familiar_sys::wallpaper::load(&path, 320) else { return };
            blur(&mut pic.rgba, pic.width as usize, pic.height as usize, 6);
            crate::app::post(move |a| {
                if a.lock.is_visible() {
                    a.lock.ui.set_backdrop(ui::image_from_rgba(pic.width, pic.height, &pic.rgba));
                    a.lock.ui.set_has_backdrop(true);
                }
            });
        });
        if let Err(e) = spawned {
            log::warn!("lock backdrop: {e}");
        }
    }

    fn unlock(&self) {
        // Ignore input during the splash and while already leaving.
        if self.ui.get_phase() != 1 || self.ui.get_leaving() {
            return;
        }
        self.ui.set_leaving(true);
        self.leave_timer.start(TimerMode::SingleShot, LEAVE, || {
            with_app(|a| {
                a.lock.visible.set(false);
                let _ = a.lock.ui.hide();
                // Drop the wallpaper pixels while hidden.
                a.lock.ui.set_backdrop(slint::Image::default());
                a.lock.ui.set_has_backdrop(false);
            })
        });
    }

    /// Big dot-matrix time and the date.
    pub fn refresh_clock(&self, app: &App) {
        let now = clock::now();
        let text = clock::time_string();
        let digits = if text.chars().any(char::is_alphabetic) {
            let h = match now.hour % 12 {
                0 => 12,
                h => h,
            };
            format!("{h}:{:02}", now.minute)
        } else {
            format!("{:02}:{:02}", now.hour, now.minute)
        };
        let scale = app.monitor.get().scale;
        let pitch = (16.0 * scale).round() as u32;
        let mut m = Matrix::new(font::text_width(&digits), font::GLYPH_H);
        m.draw_text(&digits, 0, 0, 1.0);
        let style = DotStyle { pitch, radius: 0.4, on: Rgba(255, 255, 255, 255), off: Rgba(255, 255, 255, 22) };
        let (w, h, px) = raster::render(&m, &style);
        self.ui.set_clock_dots(ui::image_from_rgba(w, h, &px));
        self.ui.set_clock_width(w as f32 / scale);
        self.ui.set_clock_height(h as f32 / scale);
        let date = crate::i18n::date(crate::i18n::long_date_picture());
        let mut c = date.chars();
        let date: String = c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default();
        self.ui.set_date(date.into());
    }

    pub fn set_battery(&self, b: Battery) {
        self.ui.set_battery(b.percent.map(i32::from).unwrap_or(-1));
        self.ui.set_charging(b.charging);
    }

    pub fn set_media(&self, media: Option<&NowPlaying>) {
        self.ui.set_has_media(media.is_some());
        if let Some(m) = media {
            self.ui.set_playing(m.playing);
            self.ui.set_track(m.title.as_str().into());
            self.ui.set_artist(m.artist.as_str().into());
            match &m.artwork {
                Some(a) => {
                    self.ui.set_artwork(ui::image_from_rgba(a.width, a.height, &a.rgba));
                    self.ui.set_has_artwork(true);
                }
                None => self.ui.set_has_artwork(false),
            }
        }
    }
}

/// Three box-blur passes (close to a Gaussian) over RGBA pixels, in place.
fn blur(px: &mut [u8], w: usize, h: usize, radius: usize) {
    if w == 0 || h == 0 || radius == 0 {
        return;
    }
    let mut tmp = px.to_vec();
    for _ in 0..3 {
        box_pass(px, &mut tmp, w, h, radius, true);
        box_pass(&tmp, px, w, h, radius, false);
    }
}

/// One horizontal (or vertical) running-average pass from `src` into `dst`.
fn box_pass(src: &[u8], dst: &mut [u8], w: usize, h: usize, r: usize, horizontal: bool) {
    let (lines, len) = if horizontal { (h, w) } else { (w, h) };
    let at = |line: usize, i: usize| if horizontal { (line * w + i) * 4 } else { (i * w + line) * 4 };
    for line in 0..lines {
        let mut sum = [0u32; 4];
        let clamp = |i: isize| i.clamp(0, len as isize - 1) as usize;
        for i in -(r as isize)..=(r as isize) {
            let p = at(line, clamp(i));
            for c in 0..4 {
                sum[c] += u32::from(src[p + c]);
            }
        }
        let n = (2 * r + 1) as u32;
        for i in 0..len {
            let p = at(line, i);
            for c in 0..4 {
                dst[p + c] = (sum[c] / n) as u8;
            }
            let out = at(line, clamp(i as isize - r as isize));
            let inn = at(line, clamp(i as isize + r as isize + 1));
            for c in 0..4 {
                sum[c] = sum[c] + u32::from(src[inn + c]) - u32::from(src[out + c]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::blur;

    #[test]
    fn blur_spreads_a_bright_dot_and_keeps_flat_areas() {
        let (w, h) = (21, 21);
        let mut px = vec![0u8; w * h * 4];
        let centre = (10 * w + 10) * 4;
        px[centre..centre + 4].copy_from_slice(&[255, 255, 255, 255]);
        blur(&mut px, w, h, 3);
        assert!(px[centre] < 255 && px[centre] > 0);
        assert!(px[(10 * w + 13) * 4] > 0, "light reached a neighbour");
        let mut flat = vec![120u8; 8 * 8 * 4];
        blur(&mut flat, 8, 8, 2);
        assert!(flat.iter().all(|v| *v == 120));
    }
}
