//! The Dynamic Island: a pill at the top centre that expands on hover into
//! a Now Playing card with transport controls, a seek bar and a live
//! spectrum. It also shows short alert pills (audio device connected,
//! focus finished) and runs the Focus (Pomodoro) timer.
//!
//! The native window is only as large as the pill so it never blocks clicks
//! on the top bar. On hover it grows to the expanded size first, then the
//! pill animates inside; on leave the pill animates back and the window
//! shrinks once the animation has finished.

use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use familiar_sys::Hwnd;
use familiar_sys::media::{self, MediaCommand, NowPlaying, Timeline};
use familiar_sys::privacy::PrivacyState;
use familiar_sys::window::{self, Rect, SurfaceOptions};
use slint::{ComponentHandle, Timer, TimerMode};
use windows::Win32::Foundation::POINT;
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

use crate::app::{App, with_app};
use crate::i18n::t;
use crate::ui::{self, IslandWindow};

const EXPANDED_W: f32 = 400.0;
const EXPANDED_H: f32 = 214.0;
const MEDIA_W: f32 = 200.0;
const IDLE_W: f32 = 96.0;
const FOCUS_W: f32 = 150.0;
const ALERT_W: f32 = 300.0;
const ALERT_TIME: Duration = Duration::from_millis(2800);
const DEVICE_POLL: Duration = Duration::from_secs(2);

/// Segoe Fluent glyphs for the alert pills.
const GLYPH_HEADPHONES: &str = "\u{E7F6}";
const GLYPH_SPEAKER_OFF: &str = "\u{E74F}";
const GLYPH_TIMER: &str = "\u{E916}";
const TOP_GAP: f32 = 3.0;

pub struct Island {
    pub ui: IslandWindow,
    hwnd: Cell<Option<Hwnd>>,
    expanded: Cell<bool>,
    shrink: Timer,
    probe: Timer,
    /// Moves the seek bar along while expanded.
    progress: Timer,
    timeline: Cell<Option<(Timeline, bool)>>,
    alert_timer: Timer,
    devices: Timer,
    /// Audio outputs seen at the last poll (None before the first one).
    known_outputs: RefCell<Option<(Vec<String>, Option<String>)>>,
    focus_timer: Timer,
    focus_end: Cell<Option<Instant>>,
    /// Keeps the island above the top bar (see `keep_on_top`).
    z_guard: Timer,
}

impl Island {
    pub fn new() -> Result<Self, slint::PlatformError> {
        let ui = IslandWindow::new()?;
        ui.on_hover_changed(|hovered| {
            with_app(|a| {
                if let Some(i) = a.island.as_ref() {
                    i.on_hover(a, hovered);
                }
            })
        });
        ui.on_play_pause(|| with_app(|a| a.media_command(MediaCommand::PlayPause)));
        ui.on_next(|| with_app(|a| a.media_command(MediaCommand::Next)));
        ui.on_previous(|| with_app(|a| a.media_command(MediaCommand::Previous)));
        ui.on_open_source(|| with_app(|a| a.focus_media_source()));
        ui.on_seek(|f| {
            with_app(|a| {
                if let Some(i) = a.island.as_ref() {
                    i.seek(a, f);
                }
            })
        });
        ui.on_focus_toggle(|| {
            with_app(|a| {
                if let Some(i) = a.island.as_ref() {
                    i.toggle_focus(a);
                }
            })
        });
        Ok(Self {
            ui,
            hwnd: Cell::new(None),
            expanded: Cell::new(false),
            shrink: Timer::default(),
            probe: Timer::default(),
            progress: Timer::default(),
            timeline: Cell::new(None),
            alert_timer: Timer::default(),
            devices: Timer::default(),
            known_outputs: RefCell::new(None),
            focus_timer: Timer::default(),
            focus_end: Cell::new(None),
            z_guard: Timer::default(),
        })
    }

    fn collapsed_size(&self, app: &App) -> (f32, f32) {
        let bar = app.theme.borrow().metrics.top_bar_height as f32;
        let w = if !self.ui.get_alert_text().is_empty() {
            ALERT_W
        } else if self.ui.get_has_media() {
            MEDIA_W
        } else if self.ui.get_focus_active() {
            FOCUS_W
        } else {
            IDLE_W
        };
        (w, (bar - 2.0 * TOP_GAP).max(18.0))
    }

    fn rect(&self, app: &App, expanded: bool) -> Rect {
        let mon = app.monitor.get();
        // Room for the spring easing to overshoot without being clipped.
        let (w, h) = if expanded { (EXPANDED_W + 24.0, EXPANDED_H + 12.0) } else { self.collapsed_size(app) };
        let (w, h) = ((w * mon.scale).round() as i32, (h * mon.scale).round() as i32);
        Rect { x: mon.bounds.x + (mon.bounds.w - w) / 2, y: mon.bounds.y + (TOP_GAP * mon.scale).round() as i32, w, h }
    }

    fn sync_sizes(&self, app: &App) {
        let (w, h) = self.collapsed_size(app);
        self.ui.set_collapsed_width(w);
        self.ui.set_collapsed_height(h);
        self.ui.set_expanded_width(EXPANDED_W);
        self.ui.set_expanded_height(EXPANDED_H);
    }

    pub fn show(&self, app: &App) {
        self.sync_sizes(app);
        let r = self.rect(app, false);
        let w = self.ui.window();
        w.set_position(slint::PhysicalPosition::new(r.x, r.y));
        w.set_size(slint::PhysicalSize::new(r.w as u32, r.h as u32));
        if let Err(e) = self.ui.show() {
            log::error!("island: {e}");
            return;
        }
        let Some(hwnd) = ui::hwnd_of(w) else { return };
        self.hwnd.set(Some(hwnd));
        window::apply_surface(
            hwnd.get(),
            SurfaceOptions { tool_window: true, no_activate: true, topmost: true, click_through: false },
        );
        // The pill paints its own black background; no DWM material.
        let h = hwnd.get();
        familiar_sys::dwm::disable_transitions(h);
        familiar_sys::dwm::hide_border(h);
        familiar_sys::dwm::set_rounded(h, false);
        window::raise_topmost(h);
        self.z_guard.start(TimerMode::Repeated, Duration::from_millis(1500), || {
            with_app(|a| {
                if let Some(i) = a.island.as_ref() {
                    i.keep_on_top(a);
                }
            })
        });
        if app.cfg.borrow().island.device_alerts {
            self.poll_devices(app);
            self.devices.start(TimerMode::Repeated, DEVICE_POLL, || {
                with_app(|a| {
                    if let Some(i) = a.island.as_ref() {
                        i.poll_devices(a);
                    }
                })
            });
        }
        self.ui.set_focus_text(format_clock(focus_length(app)).into());
    }

    /// Expands on hover. Leaving is *not* taken from hover events: growing
    /// the layered window leaves its new area transparent for a frame, and
    /// Windows then reports the pointer as gone, which used to snap the
    /// island shut right after opening. Instead, while expanded, the real
    /// cursor position is probed and the island closes once it is outside.
    fn on_hover(&self, app: &App, hovered: bool) {
        if self.hwnd.get().is_none() || !hovered {
            return;
        }
        self.shrink.stop();
        if !self.expanded.replace(true) {
            ui::set_bounds(self.ui.window(), self.rect(app, true));
        }
        self.ui.set_expanded(true);
        self.update_progress();
        self.progress.start(TimerMode::Repeated, Duration::from_millis(250), || {
            with_app(|a| {
                if let Some(i) = a.island.as_ref() {
                    i.update_progress();
                }
            })
        });
        self.probe.start(TimerMode::Repeated, Duration::from_millis(120), || {
            with_app(|a| {
                if let Some(i) = a.island.as_ref() {
                    i.check_pointer(a);
                }
            })
        });
    }

    fn check_pointer(&self, app: &App) {
        let r = self.rect(app, true);
        let mut p = POINT::default();
        let inside =
            unsafe { GetCursorPos(&mut p) }.is_ok() && p.x >= r.x && p.x < r.x + r.w && p.y >= r.y && p.y < r.y + r.h;
        if inside {
            return;
        }
        self.probe.stop();
        self.progress.stop();
        self.ui.set_expanded(false);
        self.shrink.start(TimerMode::SingleShot, Duration::from_millis(460), || {
            with_app(|a| {
                if let Some(i) = a.island.as_ref() {
                    i.collapse_window(a);
                }
            })
        });
    }

    fn collapse_window(&self, app: &App) {
        if self.ui.get_expanded() {
            return;
        }
        self.expanded.set(false);
        if self.hwnd.get().is_some() {
            ui::set_bounds(self.ui.window(), self.rect(app, false));
        }
    }

    pub fn relayout(&self, app: &App) {
        self.sync_sizes(app);
        if self.hwnd.get().is_some() {
            ui::set_bounds(self.ui.window(), self.rect(app, self.expanded.get()));
        }
    }

    pub fn apply_theme(&self, app: &App) {
        ui::push_theme(&self.ui, &app.theme.borrow());
        self.relayout(app);
    }

    pub fn set_media(&self, app: &App, media: Option<&NowPlaying>) {
        let had = self.ui.get_has_media();
        match media {
            Some(m) => {
                self.ui.set_has_media(true);
                self.ui.set_playing(m.playing);
                self.ui.set_track(m.title.as_str().into());
                self.ui.set_artist(m.artist.as_str().into());
                self.timeline.set(m.timeline.map(|t| (t, m.playing)));
                match &m.artwork {
                    Some(a) => {
                        self.ui.set_artwork(ui::image_from_rgba(a.width, a.height, &a.rgba));
                        self.ui.set_has_artwork(true);
                    }
                    None => self.ui.set_has_artwork(false),
                }
            }
            None => {
                self.ui.set_has_media(false);
                self.ui.set_playing(false);
                self.ui.set_track("".into());
                self.ui.set_artist("".into());
                self.ui.set_has_artwork(false);
                self.timeline.set(None);
            }
        }
        self.update_progress();
        if had != self.ui.get_has_media() {
            self.relayout(app);
        }
    }

    /// Refreshes the seek bar and the elapsed / total labels.
    fn update_progress(&self) {
        match self.timeline.get() {
            Some((t, playing)) if t.duration_ms > 0 => {
                let pos = t.position_now(playing);
                self.ui.set_has_timeline(true);
                self.ui.set_progress(pos as f32 / t.duration_ms as f32);
                self.ui.set_elapsed_text(media::format_time(pos).into());
                self.ui.set_total_text(media::format_time(t.duration_ms).into());
            }
            _ => {
                self.ui.set_has_timeline(false);
                self.ui.set_progress(0.0);
            }
        }
    }

    fn seek(&self, app: &App, fraction: f32) {
        let Some((t, playing)) = self.timeline.get() else { return };
        let ms = (t.duration_ms as f32 * fraction.clamp(0.0, 1.0)) as u64;
        // Show the new position straight away; the app confirms it later.
        self.timeline.set(Some((Timeline { position_ms: ms, read_at: Instant::now(), ..t }, playing)));
        self.update_progress();
        app.media_command(MediaCommand::Seek(ms));
    }

    /// Shows a short message in the collapsed pill.
    pub fn alert(&self, app: &App, glyph: &str, text: &str) {
        self.ui.set_alert_glyph(glyph.into());
        self.ui.set_alert_text(text.into());
        self.relayout(app);
        self.alert_timer.start(TimerMode::SingleShot, ALERT_TIME, || {
            with_app(|a| {
                if let Some(i) = a.island.as_ref() {
                    i.ui.set_alert_text("".into());
                    i.relayout(a);
                }
            })
        });
    }

    /// Announces audio outputs that appeared or went away (headphones,
    /// Bluetooth speakers) and changes of the default output.
    fn poll_devices(&self, app: &App) {
        let (names, default) = familiar_sys::controls::audio_outputs();
        let previous = self.known_outputs.replace(Some((names.clone(), default.clone())));
        let Some((old_names, old_default)) = previous else { return };
        if let Some(added) = names.iter().find(|n| !old_names.contains(n)) {
            self.alert(app, GLYPH_HEADPHONES, &format!("{} · {added}", t("Conectado")));
        } else if let Some(gone) = old_names.iter().find(|n| !names.contains(n)) {
            self.alert(app, GLYPH_SPEAKER_OFF, &format!("{} · {gone}", t("Desconectado")));
        } else if default != old_default
            && let Some(d) = &default
        {
            self.alert(app, GLYPH_HEADPHONES, &format!("{} · {d}", t("Salida")));
        }
    }

    /// Starts or stops a Focus session.
    fn toggle_focus(&self, app: &App) {
        if self.focus_end.take().is_some() {
            self.focus_timer.stop();
            self.ui.set_focus_active(false);
            self.ui.set_focus_text(format_clock(focus_length(app)).into());
            self.relayout(app);
            return;
        }
        self.focus_end.set(Some(Instant::now() + focus_length(app)));
        self.ui.set_focus_active(true);
        self.tick_focus(app);
        self.relayout(app);
        self.focus_timer.start(TimerMode::Repeated, Duration::from_millis(500), || {
            with_app(|a| {
                if let Some(i) = a.island.as_ref() {
                    i.tick_focus(a);
                }
            })
        });
    }

    fn tick_focus(&self, app: &App) {
        let Some(end) = self.focus_end.get() else { return };
        let left = end.saturating_duration_since(Instant::now());
        self.ui.set_focus_text(format_clock(left).into());
        if left.is_zero() {
            self.focus_end.set(None);
            self.focus_timer.stop();
            self.ui.set_focus_active(false);
            self.ui.set_focus_text(format_clock(focus_length(app)).into());
            familiar_sys::util::chime();
            self.alert(app, GLYPH_TIMER, t("Focus terminado · tómate un descanso"));
        }
    }

    /// The island sits on top of the top bar. Both are topmost, and the bar
    /// is raised again by AppBar and work-area changes; once it is above the
    /// island it takes every mouse event and the island never opens. So the
    /// island is put back on top after such changes and on a slow timer.
    pub fn keep_on_top(&self, app: &App) {
        if app.lock.is_visible() || app.is_calm() {
            return;
        }
        if let Some(h) = self.hwnd.get() {
            window::raise_topmost(h.get());
        }
    }

    pub fn set_privacy(&self, p: &PrivacyState) {
        self.ui.set_mic(p.mic_active());
        self.ui.set_cam(p.cam_active());
    }

    pub fn set_spectrum(&self, image: slint::Image) {
        self.ui.set_spectrum(image);
    }

    pub fn set_visible(&self, visible: bool) {
        ui::set_native_visible(self.hwnd.get(), visible);
    }
}

fn focus_length(app: &App) -> Duration {
    Duration::from_secs(u64::from(app.cfg.borrow().island.focus_minutes.clamp(1, 180)) * 60)
}

/// `mm:ss`, rounding up so a running timer never shows 00:00 early.
fn format_clock(d: Duration) -> String {
    let s = d.as_millis().div_ceil(1000) as u64;
    format!("{:02}:{:02}", s / 60, s % 60)
}

#[cfg(test)]
mod tests {
    use super::format_clock;
    use std::time::Duration;

    #[test]
    fn focus_clock_rounds_up() {
        assert_eq!(format_clock(Duration::from_secs(25 * 60)), "25:00");
        assert_eq!(format_clock(Duration::from_millis(59_001)), "01:00");
        assert_eq!(format_clock(Duration::from_millis(1)), "00:01");
        assert_eq!(format_clock(Duration::ZERO), "00:00");
    }
}
