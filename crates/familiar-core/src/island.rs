//! The Dynamic Island: a pill at the top centre that expands on hover into
//! a Now Playing card with transport controls and a live spectrum.
//!
//! The native window is only as large as the pill so it never blocks clicks
//! on the top bar. On hover it grows to the expanded size first, then the
//! pill animates inside; on leave the pill animates back and the window
//! shrinks once the animation has finished.

use std::cell::Cell;
use std::time::Duration;

use familiar_sys::Hwnd;
use familiar_sys::media::{MediaCommand, NowPlaying};
use familiar_sys::privacy::PrivacyState;
use familiar_sys::window::{self, Rect, SurfaceOptions};
use slint::{ComponentHandle, Timer, TimerMode};

use crate::app::{App, with_app};
use crate::ui::{self, IslandWindow};

const EXPANDED_W: f32 = 380.0;
const EXPANDED_H: f32 = 148.0;
const MEDIA_W: f32 = 210.0;
const IDLE_W: f32 = 120.0;
const TOP_GAP: f32 = 3.0;

pub struct Island {
    pub ui: IslandWindow,
    hwnd: Cell<Option<Hwnd>>,
    expanded: Cell<bool>,
    shrink: Timer,
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
        Ok(Self { ui, hwnd: Cell::new(None), expanded: Cell::new(false), shrink: Timer::default() })
    }

    fn collapsed_size(&self, app: &App) -> (f32, f32) {
        let bar = app.theme.borrow().metrics.top_bar_height as f32;
        let w = if self.ui.get_has_media() { MEDIA_W } else { IDLE_W };
        (w, (bar - 2.0 * TOP_GAP).max(18.0))
    }

    fn rect(&self, app: &App, expanded: bool) -> Rect {
        let mon = app.monitor.get();
        let (w, h) = if expanded { (EXPANDED_W, EXPANDED_H) } else { self.collapsed_size(app) };
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
    }

    fn on_hover(&self, app: &App, hovered: bool) {
        if self.hwnd.get().is_none() {
            return;
        }
        if hovered {
            self.shrink.stop();
            if !self.expanded.replace(true) {
                ui::set_bounds(self.ui.window(), self.rect(app, true));
            }
            self.ui.set_expanded(true);
        } else {
            self.ui.set_expanded(false);
            self.shrink.start(TimerMode::SingleShot, Duration::from_millis(380), || {
                with_app(|a| {
                    if let Some(i) = a.island.as_ref() {
                        i.collapse_window(a);
                    }
                })
            });
        }
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
            }
        }
        if had != self.ui.get_has_media() {
            self.relayout(app);
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
