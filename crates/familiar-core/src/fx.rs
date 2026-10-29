//! Cursor effects: click ripples and the shake-to-find halo, drawn in a
//! small click-through window that follows the event position.

use std::cell::Cell;
use std::time::Duration;

use familiar_sys::Hwnd;
use familiar_sys::mouse::MouseFx;
use familiar_sys::window::{self, Rect, SurfaceOptions};
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
