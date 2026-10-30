//! W7: the Control Center — Wi-Fi, Bluetooth, now playing, brightness,
//! volume, light/dark and shortcuts, dropping down under the top bar.

use std::cell::Cell;
use std::rc::Rc;

use familiar_sys::controls::{self, Radios};
use familiar_sys::media::{MediaCommand, NowPlaying};
use familiar_sys::window::{self, SurfaceOptions};
use familiar_sys::{Hwnd, subclass, windows_list};
use slint::ComponentHandle;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{WA_INACTIVE, WM_ACTIVATE};

use crate::app::{App, post, with_app};
use crate::ui::{self, ControlCenterWindow};

const WIDTH: f32 = 340.0;
const HEIGHT: f32 = 530.0;

pub struct ControlCenter {
    pub ui: ControlCenterWindow,
    hwnd: Cell<Option<Hwnd>>,
    visible: Cell<bool>,
}

/// Runs `f` on a worker thread (radios and WMI block).
fn background(f: impl FnOnce() + Send + 'static) {
    if let Err(e) = std::thread::Builder::new().name("control-center".into()).spawn(f) {
        log::warn!("control center thread: {e}");
    }
}

impl ControlCenter {
    pub fn new() -> Result<Self, slint::PlatformError> {
        let ui = ControlCenterWindow::new()?;
        ui.on_toggle_wifi(|| with_app(|a| a.control_center.toggle_radio(true)));
        ui.on_toggle_bluetooth(|| with_app(|a| a.control_center.toggle_radio(false)));
        ui.on_volume_moved(|v| {
            controls::set_volume(v);
            with_app(|a| a.control_center.ui.set_muted(false));
        });
        ui.on_toggle_mute(|| {
            with_app(|a| {
                let muted = !a.control_center.ui.get_muted();
                controls::set_mute(muted);
                a.control_center.ui.set_muted(muted);
            })
        });
        ui.on_brightness_released(|b| {
            let pct = (b * 100.0).round() as u32;
            background(move || controls::set_brightness(pct));
        });
        ui.on_set_dark(|dark| with_app(|a| a.set_dark_mode(dark)));
        ui.on_open_system_settings(|| {
            with_app(|a| {
                a.control_center.hide();
                let _ = familiar_sys::apps::launch("ms-settings:");
            })
        });
        ui.on_open_familiar_settings(|| {
            with_app(|a| {
                a.control_center.hide();
                a.settings.show(a);
            })
        });
        ui.on_lock(|| {
            with_app(|a| {
                a.control_center.hide();
                a.lock.show(a);
            })
        });
        ui.on_previous(|| with_app(|a| a.media_command(MediaCommand::Previous)));
        ui.on_play_pause(|| with_app(|a| a.media_command(MediaCommand::PlayPause)));
        ui.on_next(|| with_app(|a| a.media_command(MediaCommand::Next)));
        Ok(Self { ui, hwnd: Cell::new(None), visible: Cell::new(false) })
    }

    pub fn toggle(&self, app: &App) {
        if self.visible.get() {
            self.hide();
        } else if !app.is_calm() {
            self.show(app);
        }
    }

    pub fn show(&self, app: &App) {
        ui::push_theme(&self.ui, &app.theme.borrow());
        self.refresh(app);
        let mon = app.monitor.get();
        let (w, h) = ((WIDTH * mon.scale) as i32, (HEIGHT * mon.scale) as i32);
        let x = mon.work.x + mon.work.w - w - (10.0 * mon.scale) as i32;
        let y = mon.work.y + (6.0 * mon.scale) as i32;
        let win = self.ui.window();
        win.set_position(slint::PhysicalPosition::new(x, y));
        win.set_size(slint::PhysicalSize::new(w as u32, h as u32));
        self.ui.set_shown(false);
        if let Err(e) = self.ui.show() {
            log::error!("control center: {e}");
            return;
        }
        self.visible.set(true);
        if let Some(hwnd) = ui::hwnd_of(win) {
            if self.hwnd.replace(Some(hwnd)).is_none() {
                window::apply_surface(
                    hwnd.get(),
                    SurfaceOptions { tool_window: true, no_activate: false, topmost: true, click_through: false },
                );
                // Close when the user clicks elsewhere.
                subclass::add(
                    hwnd.get(),
                    Rc::new(|_, msg, wp: WPARAM, _: LPARAM| {
                        if msg == WM_ACTIVATE && (wp.0 & 0xFFFF) as u32 == WA_INACTIVE {
                            slint::Timer::single_shot(std::time::Duration::ZERO, || {
                                with_app(|a| a.control_center.hide())
                            });
                        }
                        None
                    }),
                );
            }
            windows_list::activate(hwnd);
        }
        slint::Timer::single_shot(std::time::Duration::from_millis(10), || {
            with_app(|a| a.control_center.ui.set_shown(true))
        });
    }

    pub fn hide(&self) {
        if self.visible.replace(false) {
            let _ = self.ui.hide();
        }
    }

    /// Volume now; radios and brightness from a worker thread.
    fn refresh(&self, app: &App) {
        if let Some((v, muted)) = controls::volume() {
            self.ui.set_volume(v);
            self.ui.set_muted(muted);
        }
        self.ui.set_dark(app.theme.borrow().colors.background_tint.luminance() < 0.5);
        background(|| {
            let radios = controls::radios();
            post(move |a| a.control_center.set_radios(&radios));
            let brightness = controls::brightness();
            post(move |a| {
                a.control_center.ui.set_has_brightness(brightness.is_some());
                if let Some(b) = brightness {
                    a.control_center.ui.set_brightness(b as f32 / 100.0);
                }
            });
        });
    }

    fn set_radios(&self, r: &Radios) {
        self.ui.set_wifi_available(r.wifi.is_some());
        self.ui.set_wifi_on(r.wifi.unwrap_or(false));
        self.ui.set_bt_available(r.bluetooth.is_some());
        self.ui.set_bt_on(r.bluetooth.unwrap_or(false));
        self.ui.set_network(r.network.as_str().into());
    }

    fn toggle_radio(&self, wifi: bool) {
        let on = if wifi { !self.ui.get_wifi_on() } else { !self.ui.get_bt_on() };
        // Optimistic: flip the tile now, confirm from the system after.
        if wifi {
            self.ui.set_wifi_on(on);
        } else {
            self.ui.set_bt_on(on);
        }
        background(move || {
            controls::set_radio(wifi, on);
            let radios = controls::radios();
            post(move |a| a.control_center.set_radios(&radios));
        });
    }

    pub fn set_media(&self, media: Option<&NowPlaying>) {
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
            None => self.ui.set_has_media(false),
        }
    }
}
