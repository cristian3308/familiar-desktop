//! W4: the on-demand launcher (Win tap or a hotkey), searching the in-memory
//! app index on every keystroke.
//!
//! Three tabs: fuzzy app search, recent documents and every app A–Z. Any
//! tab shows an inline calculator row first when the query is arithmetic
//! (`= 2*(3+4)` or `12/5`); Enter copies the result.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use familiar_store::{AppEntry, AppSearch};
use familiar_sys::icons::{self, IconSource};
use familiar_sys::recent::{self, RecentFile};
use familiar_sys::window::{self, SurfaceOptions};
use familiar_sys::{Hwnd, clipboard, subclass};
use slint::{ComponentHandle, VecModel};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{WA_INACTIVE, WM_ACTIVATE};

use crate::app::{App, with_app};
use crate::calc;
use crate::i18n::t;
use crate::ui::{self, LauncherItem, LauncherWindow};

const WIDTH: f32 = 680.0;
const HEIGHT: f32 = 640.0;
const RECENT_LIMIT: usize = 40;

const GLYPH_CALC: &str = "\u{E8EF}";
const GLYPH_FOLDER: &str = "\u{E8B7}";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    Apps,
    Recent,
    AllApps,
}

impl Tab {
    fn from_index(i: i32) -> Self {
        match i.rem_euclid(3) {
            1 => Tab::Recent,
            2 => Tab::AllApps,
            _ => Tab::Apps,
        }
    }

    fn index(self) -> i32 {
        match self {
            Tab::Apps => 0,
            Tab::Recent => 1,
            Tab::AllApps => 2,
        }
    }
}

/// What a result row does when activated.
enum Row {
    App(usize),
    File(PathBuf),
    Calc(String),
}

pub struct Launcher {
    pub ui: LauncherWindow,
    search: RefCell<AppSearch>,
    results: RefCell<Vec<Row>>,
    model: Rc<VecModel<LauncherItem>>,
    hwnd: Cell<Option<Hwnd>>,
    visible: Cell<bool>,
    tab: Cell<Tab>,
    recent: RefCell<Vec<RecentFile>>,
    /// Shell icons of recent files, extracted once per session.
    file_icons: RefCell<HashMap<PathBuf, slint::Image>>,
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
        ui.on_tab_changed(|i| with_app(|a| a.launcher.set_tab(a, Tab::from_index(i))));
        Ok(Self {
            ui,
            search: RefCell::new(AppSearch::new(Vec::new())),
            results: RefCell::new(Vec::new()),
            model,
            hwnd: Cell::new(None),
            visible: Cell::new(false),
            tab: Cell::new(Tab::Apps),
            recent: RefCell::new(Vec::new()),
            file_icons: RefCell::new(HashMap::new()),
        })
    }

    pub fn set_apps(&self, apps: Vec<AppEntry>) {
        *self.search.borrow_mut() = AppSearch::new(apps);
    }

    pub fn app_count(&self) -> usize {
        self.search.borrow().apps().len()
    }

    pub fn find_by_exe(&self, exe: &str) -> Option<AppEntry> {
        // Shortcuts that open a folder or a script point at a generic host
        // (e.g. "Windows Software Development Kit" → explorer.exe); those
        // must not name every window of that host.
        if is_generic_host(exe) {
            return None;
        }
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

    /// Store app by package family ("Claude_pzs8sxrjxfjjc").
    pub fn find_by_family(&self, family: &str) -> Option<AppEntry> {
        let prefix = format!("{}!", family.to_ascii_lowercase());
        self.search.borrow().apps().iter().find(|a| a.launch_target.to_ascii_lowercase().starts_with(&prefix)).cloned()
    }

    pub fn find_by_name(&self, name: &str) -> Option<AppEntry> {
        self.search.borrow().apps().iter().find(|a| a.name.eq_ignore_ascii_case(name)).cloned()
    }

    pub fn toggle(&self, app: &App) {
        if self.visible.get() {
            self.hide();
        } else if !app.is_calm() && !app.lock.is_visible() {
            // Never pop over a full-screen game or the lock face.
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
        self.ui.set_shown(false);
        self.tab.set(Tab::Apps);
        self.ui.set_tab(0);
        *self.recent.borrow_mut() = recent::recent_files(RECENT_LIMIT);
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
        // Flip on the next turn of the loop so the entry animation runs.
        slint::Timer::single_shot(std::time::Duration::from_millis(10), || with_app(|a| a.launcher.ui.set_shown(true)));
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

    fn set_tab(&self, app: &App, tab: Tab) {
        self.tab.set(tab);
        self.ui.set_tab(tab.index());
        let query = self.ui.get_query();
        self.refresh(app, &query);
        self.ui.invoke_focus_search();
    }

    fn refresh(&self, app: &App, query: &str) {
        let mut rows: Vec<Row> = Vec::new();
        let mut items: Vec<LauncherItem> = Vec::new();
        if let Some(v) = calc::evaluate(query) {
            let text = calc::format(v);
            items.push(LauncherItem {
                icon: slint::Image::default(),
                glyph: GLYPH_CALC.into(),
                name: format!("= {text}").into(),
                detail: t("Calculadora").into(),
                hint: t("COPIAR ↵").into(),
            });
            rows.push(Row::Calc(text));
        }
        match self.tab.get() {
            Tab::Apps => {
                let limit = app.cfg.borrow().launcher.max_results;
                let search = self.search.borrow();
                for h in search.search(query, limit, now_unix()) {
                    if let Some(a) = search.get(h.index) {
                        items.push(app_item(a, a.category.clone().unwrap_or_default()));
                        rows.push(Row::App(h.index));
                    }
                }
            }
            Tab::AllApps => {
                let search = self.search.borrow();
                for i in alphabetical(search.apps(), query) {
                    let a = &search.apps()[i];
                    items.push(app_item(a, initial(&a.name)));
                    rows.push(Row::App(i));
                }
            }
            Tab::Recent => {
                let needle = query.trim().to_lowercase();
                let recent = self.recent.borrow();
                for f in recent.iter().filter(|f| needle.is_empty() || f.name.to_lowercase().contains(&needle)) {
                    items.push(self.file_item(f));
                    rows.push(Row::File(f.link.clone()));
                }
            }
        }
        *self.results.borrow_mut() = rows;
        self.model.set_vec(items);
        self.ui.set_selected(0);
        self.ui.invoke_scroll_top();
    }

    fn file_item(&self, f: &RecentFile) -> LauncherItem {
        let icon = if f.is_folder {
            slint::Image::default()
        } else {
            self.file_icons
                .borrow_mut()
                .entry(f.link.clone())
                .or_insert_with(|| {
                    icons::extract(IconSource::Path(&f.link.to_string_lossy()), 32)
                        .map(|i| ui::image_from_rgba(i.width, i.height, &i.rgba))
                        .unwrap_or_default()
                })
                .clone()
        };
        LauncherItem {
            icon,
            glyph: if f.is_folder { GLYPH_FOLDER.into() } else { "".into() },
            name: f.name.as_str().into(),
            detail: (if f.is_folder { t("Carpeta reciente") } else { t("Archivo reciente") }).into(),
            hint: t("ABRIR ↵").into(),
        }
    }

    fn activate(&self, app: &App, row: i32) {
        enum Action {
            Launch(String),
            Open(PathBuf),
            Copy(String),
        }
        let action = {
            let results = self.results.borrow();
            let search = self.search.borrow();
            match results.get(row.max(0) as usize) {
                Some(Row::App(i)) => search.get(*i).map(|a| Action::Launch(a.launch_target.clone())),
                Some(Row::File(p)) => Some(Action::Open(p.clone())),
                Some(Row::Calc(v)) => Some(Action::Copy(v.clone())),
                None => None,
            }
        };
        let Some(action) = action else { return };
        self.hide();
        match action {
            Action::Launch(target) => {
                app.launch(&target);
                self.search.borrow_mut().note_launch(&target, now_unix());
            }
            Action::Open(path) => app.launch(&path.to_string_lossy()),
            Action::Copy(value) => {
                if let Err(e) = clipboard::set_text(&value) {
                    log::warn!("copying {value}: {e}");
                }
            }
        }
    }
}

fn app_item(a: &AppEntry, detail: String) -> LauncherItem {
    LauncherItem {
        icon: ui::load_icon(std::path::Path::new(&a.icon_cache_path)),
        glyph: "".into(),
        name: a.name.as_str().into(),
        detail: detail.into(),
        hint: t("ABRIR ↵").into(),
    }
}

/// Indices of `apps` whose name contains `query`, sorted by name.
fn alphabetical(apps: &[AppEntry], query: &str) -> Vec<usize> {
    let needle = query.trim().to_lowercase();
    let mut idx: Vec<usize> =
        (0..apps.len()).filter(|i| needle.is_empty() || apps[*i].name.to_lowercase().contains(&needle)).collect();
    idx.sort_by_cached_key(|i| apps[*i].name.to_lowercase());
    idx
}

/// Group letter for the A–Z list ("#" for digits and symbols).
fn initial(name: &str) -> String {
    match name.chars().next() {
        Some(c) if c.is_alphabetic() => c.to_uppercase().collect(),
        _ => "#".into(),
    }
}

/// Executables that host many unrelated shortcuts and windows.
pub fn is_generic_host(exe: &str) -> bool {
    let file = exe.rsplit(['\\', '/']).next().unwrap_or_default().to_ascii_lowercase();
    matches!(
        file.as_str(),
        "explorer.exe"
            | "cmd.exe"
            | "rundll32.exe"
            | "wscript.exe"
            | "cscript.exe"
            | "mshta.exe"
            | "msiexec.exe"
            | "dllhost.exe"
            | "applicationframehost.exe"
    )
}

#[cfg(test)]
mod tests {
    use super::{alphabetical, initial};
    use familiar_store::AppEntry;

    fn app(name: &str) -> AppEntry {
        AppEntry { name: name.into(), ..Default::default() }
    }

    #[test]
    fn a_to_z_sorts_and_filters() {
        let apps = vec![app("zoom"), app("Brave"), app("Calculadora"), app("audacity")];
        let names = |v: Vec<usize>| v.into_iter().map(|i| apps[i].name.clone()).collect::<Vec<_>>();
        assert_eq!(names(alphabetical(&apps, "")), ["audacity", "Brave", "Calculadora", "zoom"]);
        assert_eq!(names(alphabetical(&apps, "a")), ["audacity", "Brave", "Calculadora"]);
    }

    #[test]
    fn group_letters() {
        assert_eq!(initial("brave"), "B");
        assert_eq!(initial("7-Zip"), "#");
        assert_eq!(initial("Ñandú"), "Ñ");
    }
}
