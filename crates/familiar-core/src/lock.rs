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
