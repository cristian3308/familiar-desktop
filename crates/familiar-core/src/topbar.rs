//! W1: the top bar, registered as an AppBar so maximised windows stay below it.

use std::cell::RefCell;

use familiar_sys::appbar::{AppBar, AppBarEvent, Edge};
use familiar_sys::power::Battery;
use familiar_sys::privacy::PrivacyState;
use familiar_sys::window::{self, Rect, SurfaceOptions};
use familiar_sys::{Hwnd, clock};
use slint::ComponentHandle;

use crate::app::{App, with_app};
use crate::ui::{self, TopBarWindow};

pub struct TopBar {
    pub ui: TopBarWindow,
    appbar: RefCell<Option<AppBar>>,
    hwnd: std::cell::Cell<Option<Hwnd>>,
}

impl TopBar {
    pub fn new() -> Result<Self, slint::PlatformError> {
        let ui = TopBarWindow::new()?;
        ui.on_logo_clicked(|| with_app(|a| a.launcher.toggle(a)));
        ui.on_app_clicked(|| with_app(|a| a.launcher.toggle(a)));
        ui.on_clock_clicked(|| open("ms-actioncenter:"));
        ui.on_battery_clicked(|| open("ms-settings:batterysaver"));
        ui.on_layout_clicked(|| open("ms-settings:regionlanguage"));
        ui.on_privacy_clicked(|| open("ms-settings:privacy-microphone"));
        Ok(Self { ui, appbar: RefCell::new(None), hwnd: Default::default() })
    }

    pub fn height_px(app: &App) -> i32 {
        let t = app.theme.borrow();
        (t.metrics.top_bar_height as f32 * app.monitor.get().scale).round() as i32
    }

    pub fn show(&self, app: &App) {
        let mon = app.monitor.get().bounds;
        let h = Self::height_px(app);
        let w = self.ui.window();
        w.set_position(slint::PhysicalPosition::new(mon.x, mon.y));
        w.set_size(slint::PhysicalSize::new(mon.w as u32, h as u32));
        if let Err(e) = self.ui.show() {
            log::error!("top bar: {e}");
            return;
        }
        let Some(hwnd) = ui::hwnd_of(w) else { return };
        self.hwnd.set(Some(hwnd));
        window::apply_surface(
            hwnd.get(),
            SurfaceOptions { tool_window: true, no_activate: true, topmost: true, click_through: false },
        );
        ui::apply_material(hwnd, &app.theme.borrow(), false);
        self.register_appbar(app);
        self.refresh_clock(app);
    }

    fn register_appbar(&self, app: &App) {
        let Some(hwnd) = self.hwnd.get() else { return };
        self.appbar.borrow_mut().take();
        let mon = app.monitor.get().bounds;
        let bar = AppBar::register(hwnd.get(), Edge::Top, Self::height_px(app), mon, true, |ev| match ev {
            // Arrives inside our window procedure; act once it has returned.
            AppBarEvent::FullScreen(open) => {
                slint::Timer::single_shot(std::time::Duration::ZERO, move || with_app(|a| a.on_fullscreen_hint(open)))
            }
            AppBarEvent::Repositioned(_) => {}
        });
        match bar {
            Some((bar, rect)) => {
                log::info!("top bar reserved {rect:?}");
                *self.appbar.borrow_mut() = Some(bar);
            }
            None => log::warn!("SHAppBarMessage(ABM_NEW) failed; top bar will overlap maximised windows"),
        }
    }

    /// Explorer restarted: its AppBar list is gone, register again.
    pub fn reregister(&self, app: &App) {
        self.register_appbar(app);
    }

    pub fn relayout(&self, app: &App) {
        let mon = app.monitor.get().bounds;
        let h = Self::height_px(app);
        if let Some(bar) = self.appbar.borrow().as_ref() {
            bar.set_monitor(mon);
            bar.set_thickness(h);
        } else if let Some(hwnd) = self.hwnd.get() {
            window::set_bounds(hwnd.get(), Rect { x: mon.x, y: mon.y, w: mon.w, h });
        }
    }

    pub fn apply_theme(&self, app: &App) {
        ui::push_theme(&self.ui, &app.theme.borrow());
        if let Some(hwnd) = self.hwnd.get() {
            ui::apply_material(hwnd, &app.theme.borrow(), false);
        }
        self.relayout(app);
    }

    pub fn set_visible(&self, visible: bool) {
        ui::set_native_visible(self.hwnd.get(), visible);
    }

    pub fn refresh_clock(&self, app: &App) {
        self.ui.set_clock(clock::time_string().into());
        self.ui.set_date(clock::date_string(&app.cfg.borrow().top_bar.date_format).into());
    }

    pub fn set_battery(&self, b: Battery) {
        self.ui.set_battery(b.percent.map(i32::from).unwrap_or(-1));
        self.ui.set_charging(b.charging || (b.on_ac && b.percent == Some(100)));
    }

    pub fn set_privacy(&self, p: &PrivacyState) {
        self.ui.set_mic(p.mic_active());
        self.ui.set_cam(p.cam_active());
    }

    pub fn set_layout_code(&self, code: &str) {
        self.ui.set_layout_code(code.into());
    }

    pub fn set_app_name(&self, name: &str) {
        self.ui.set_app_name(name.into());
    }

    pub fn shutdown(&self) {
        if let Some(bar) = self.appbar.borrow_mut().take() {
            bar.unregister();
        }
    }
}

fn open(uri: &str) {
    if let Err(e) = familiar_sys::apps::launch(uri) {
        log::warn!("cannot open {uri}: {e}");
    }
}
