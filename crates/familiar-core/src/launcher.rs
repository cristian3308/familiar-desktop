//! W4: the on-demand launcher (Win tap or a hotkey), searching the in-memory
//! app index on every keystroke.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use familiar_store::{AppEntry, AppSearch};
use familiar_sys::window::{self, SurfaceOptions};
use familiar_sys::{Hwnd, subclass};
use slint::{ComponentHandle, VecModel};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{WA_INACTIVE, WM_ACTIVATE};

use crate::app::{App, with_app};
use crate::ui::{self, LauncherItem, LauncherWindow};

const WIDTH: f32 = 680.0;
const HEIGHT: f32 = 500.0;

pub struct Launcher {
    pub ui: LauncherWindow,
    search: RefCell<AppSearch>,
    results: RefCell<Vec<usize>>,
    model: Rc<VecModel<LauncherItem>>,
    hwnd: Cell<Option<Hwnd>>,
    visible: Cell<bool>,
}

fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

impl Launcher {
    pub fn new() -> Result<Self, slint::PlatformError> {
        let ui = LauncherWindow::new()?;
        let model = Rc::new(VecModel::default());
        ui.set_items(model.clone().into());
        ui.on_query_changed(|q| with_app(|a| a.launcher.refresh(a, &q)));
        ui.on_activate(|i| with_app(|a| a.launcher.activate(a, i)));
        ui.on_dismiss(|| with_app(|a| a.launcher.hide()));
        Ok(Self {
            ui,
            search: RefCell::new(AppSearch::new(Vec::new())),
            results: RefCell::new(Vec::new()),
            model,
            hwnd: Cell::new(None),
            visible: Cell::new(false),
        })
    }

    pub fn set_apps(&self, apps: Vec<AppEntry>) {
        *self.search.borrow_mut() = AppSearch::new(apps);
    }

    pub fn app_count(&self) -> usize {
        self.search.borrow().apps().len()
    }

    pub fn find_by_exe(&self, exe: &str) -> Option<AppEntry> {
        self.search
            .borrow()
            .apps()
            .iter()
            .find(|a| !a.executable_path.is_empty() && a.executable_path.eq_ignore_ascii_case(exe))
            .cloned()
    }

    pub fn find_by_target(&self, target: &str) -> Option<AppEntry> {
        self.search.borrow().apps().iter().find(|a| a.launch_target.eq_ignore_ascii_case(target)).cloned()
    }

    pub fn find_by_name(&self, name: &str) -> Option<AppEntry> {
        self.search.borrow().apps().iter().find(|a| a.name.eq_ignore_ascii_case(name)).cloned()
    }

    pub fn toggle(&self, app: &App) {
        if self.visible.get() {
            self.hide();
        } else if !app.is_calm() {
            // Never pop over a full-screen game.
            self.show(app);
        }
    }

    pub fn show(&self, app: &App) {
        let mon = app.monitor.get();
        let (w, h) = ((WIDTH * mon.scale) as i32, (HEIGHT * mon.scale) as i32);
        let x = mon.work.x + (mon.work.w - w) / 2;
        let y = mon.work.y + (mon.work.h - h) / 3;
        let win = self.ui.window();
        win.set_position(slint::PhysicalPosition::new(x, y));
        win.set_size(slint::PhysicalSize::new(w as u32, h as u32));
        self.ui.set_query("".into());
        self.refresh(app, "");
        if let Err(e) = self.ui.show() {
            log::error!("launcher: {e}");
            return;
        }
        self.visible.set(true);
        if let Some(hwnd) = ui::hwnd_of(win) {
            if self.hwnd.replace(Some(hwnd)).is_none() {
                window::apply_surface(
                    hwnd.get(),
                    SurfaceOptions { tool_window: true, no_activate: false, topmost: true, click_through: false },
                );
                ui::apply_material(hwnd, &app.theme.borrow(), false);
                ui::round_region(hwnd, win, app.theme.borrow().metrics.corner_radius as f32);
                // Close when the user clicks elsewhere.
                subclass::add(
                    hwnd.get(),
                    Rc::new(|_, msg, wp: WPARAM, _: LPARAM| {
                        if msg == WM_ACTIVATE && (wp.0 & 0xFFFF) as u32 == WA_INACTIVE {
                            slint::Timer::single_shot(std::time::Duration::ZERO, || with_app(|a| a.launcher.hide()));
                        }
                        None
                    }),
                );
            }
            familiar_sys::windows_list::activate(hwnd);
        }
        self.ui.invoke_focus_search();
    }

    pub fn hide(&self) {
        if self.visible.replace(false) {
            let _ = self.ui.hide();
        }
    }

    pub fn apply_theme(&self, app: &App) {
        ui::push_theme(&self.ui, &app.theme.borrow());
        if let Some(h) = self.hwnd.get() {
            ui::apply_material(h, &app.theme.borrow(), false);
            ui::round_region(h, self.ui.window(), app.theme.borrow().metrics.corner_radius as f32);
        }
    }

    fn refresh(&self, app: &App, query: &str) {
        let limit = app.cfg.borrow().launcher.max_results;
        let search = self.search.borrow();
        let hits = search.search(query, limit, now_unix());
        let rows: Vec<LauncherItem> = hits
            .iter()
            .filter_map(|h| search.get(h.index))
            .map(|a| LauncherItem {
                icon: ui::load_icon(std::path::Path::new(&a.icon_cache_path)),
                name: a.name.as_str().into(),
                detail: a.category.clone().unwrap_or_default().into(),
            })
            .collect();
        *self.results.borrow_mut() = hits.iter().map(|h| h.index).collect();
        self.model.set_vec(rows);
        self.ui.set_selected(0);
    }

    fn activate(&self, app: &App, row: i32) {
        let target = {
            let results = self.results.borrow();
            let search = self.search.borrow();
            results.get(row.max(0) as usize).and_then(|i| search.get(*i)).map(|a| a.launch_target.clone())
        };
        let Some(target) = target else { return };
        self.hide();
        app.launch(&target);
        self.search.borrow_mut().note_launch(&target, now_unix());
    }
}
