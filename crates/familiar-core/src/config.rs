//! `config.toml` (behaviour) and on-disk locations.
//!
//! Looks (colours, sizes, materials) live in themes; this file only holds
//! what the shell does.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// Built-in theme stem (`ember-dark`) or a path to a `.toml` file.
    pub theme: String,
    /// Hide the native taskbar while running (restored on exit or crash).
    pub hide_taskbar: bool,
    pub autostart: bool,
    /// Zoom windows out of / into their dock icon when they open or minimise.
    pub window_animations: bool,
    /// Minimise with the genie bend instead of the smooth scale.
    pub genie_effect: bool,
    /// Show the splash and lock face when Familiar starts.
    pub lock_on_start: bool,
    pub dock: DockConfig,
    pub top_bar: TopBarConfig,
    pub island: IslandConfig,
    pub widgets: WidgetsConfig,
    pub launcher: LauncherConfig,
    pub calm: CalmConfig,
    pub wallpaper: WallpaperConfig,
    /// UI language: es, en, fr or ru.
    pub language: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WallpaperConfig {
    /// Alt+W switches to the next image (off by default: apps use Alt+W).
    pub alt_w: bool,
    /// Folder of images to rotate through; empty = the current wallpaper's.
    pub folder: String,
    /// fade, circle, wipe or dots.
    pub transition: String,
}

impl Default for WallpaperConfig {
    fn default() -> Self {
        Self { alt_w: false, folder: String::new(), transition: "fade".into() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DockConfig {
    pub enabled: bool,
    /// Reserve the dock's resting height so maximised windows end above it.
    pub reserve_space: bool,
    /// Show running apps that are not pinned (after a separator).
    pub show_running: bool,
    /// Draw macOS-style icons for Explorer, Settings and terminals.
    pub custom_icons: bool,
    /// Style of the Files (Explorer) icon: classic, dot, mono, theme, glass.
    pub files_style: String,
    /// Hovering a running app lists its windows above the dock.
    pub window_previews: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TopBarConfig {
    pub enabled: bool,
    /// Date picture in Windows `GetDateFormatEx` syntax.
    pub date_format: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct IslandConfig {
    pub enabled: bool,
    /// Length of a Focus (Pomodoro) session.
    pub focus_minutes: u32,
    /// Show a pill when audio outputs are connected or removed.
    pub device_alerts: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WidgetsConfig {
    pub enabled: bool,
    /// Parent the widgets to the wallpaper layer (`WorkerW`) instead of
    /// keeping them bottom-most. Survives Win+D; sits over Wallpaper Engine.
    pub attach_to_wallpaper: bool,
    /// Widget ids to show: `calendar`, `weather`, `system`, `signal_01`,
    /// `clock_grid`.
    pub items: Vec<String>,
    /// City for the weather widget (looked up with Open-Meteo).
    pub weather_city: String,
    /// SIGNAL/01 mode: face, pulse, ripple, torus or clock.
    pub signal_mode: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LauncherConfig {
    /// `"win"` = tap the Windows key alone; otherwise a hotkey such as
    /// `"alt+space"` or `"ctrl+shift+k"`.
    pub hotkey: String,
    pub max_results: usize,
}

/// What "Calm by design" does in one scenario.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalmRule {
    /// Everything keeps running.
    #[default]
    Keep,
    /// Stop audio capture and the dot-matrix animations.
    PauseVisualizer,
    /// Also hide the desktop widgets (and, for full-screen apps, every
    /// surface: top bar, dock and island).
    PauseDesktop,
}

impl CalmRule {
    pub fn next(self) -> Self {
        match self {
            CalmRule::Keep => CalmRule::PauseVisualizer,
            CalmRule::PauseVisualizer => CalmRule::PauseDesktop,
            CalmRule::PauseDesktop => CalmRule::Keep,
        }
    }

    /// Spanish label; translated with `i18n::t`.
    pub fn label(self) -> &'static str {
        match self {
            CalmRule::Keep => "Seguir",
            CalmRule::PauseVisualizer => "Pausar visualizador",
            CalmRule::PauseDesktop => "Pausar escritorio",
        }
    }
}

/// The four situations Calm mode can react to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalmScenario {
    /// Any app window is in front of the desktop.
    Focused,
    /// The window in front is maximised.
    Maximized,
    /// A full-screen app or game is in front.
    Fullscreen,
    /// The laptop runs on battery (or battery saver is on).
    Battery,
}

impl CalmScenario {
    pub const ALL: [CalmScenario; 4] =
        [CalmScenario::Focused, CalmScenario::Maximized, CalmScenario::Fullscreen, CalmScenario::Battery];
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CalmConfig {
    pub focused: CalmRule,
    pub maximized: CalmRule,
    pub fullscreen: CalmRule,
    pub battery: CalmRule,
}

/// What is true right now, as seen by the shell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CalmInputs {
    pub focused: bool,
    pub maximized: bool,
    pub fullscreen: bool,
    pub on_battery: bool,
}

/// What to pause, resolved from every active scenario.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CalmState {
    /// Hide top bar, dock, island and widgets; drop input hooks.
    pub hide_all: bool,
    /// Hide the desktop widgets.
    pub pause_desktop: bool,
    /// Stop audio capture and dot-matrix animation.
    pub pause_visualizer: bool,
}

impl CalmConfig {
    pub fn rule(&self, s: CalmScenario) -> CalmRule {
        match s {
            CalmScenario::Focused => self.focused,
            CalmScenario::Maximized => self.maximized,
            CalmScenario::Fullscreen => self.fullscreen,
            CalmScenario::Battery => self.battery,
        }
    }

    pub fn rule_mut(&mut self, s: CalmScenario) -> &mut CalmRule {
        match s {
            CalmScenario::Focused => &mut self.focused,
            CalmScenario::Maximized => &mut self.maximized,
            CalmScenario::Fullscreen => &mut self.fullscreen,
            CalmScenario::Battery => &mut self.battery,
        }
    }

    /// The strictest rule of the scenarios that apply wins.
    pub fn evaluate(&self, now: CalmInputs) -> CalmState {
        let active = [
            (now.focused, self.focused),
            (now.maximized, self.maximized),
            (now.fullscreen, self.fullscreen),
            (now.on_battery, self.battery),
        ];
        let level = active.iter().filter(|(on, _)| *on).map(|(_, r)| *r).max().unwrap_or(CalmRule::Keep);
        CalmState {
            hide_all: now.fullscreen && self.fullscreen == CalmRule::PauseDesktop,
            pause_desktop: level == CalmRule::PauseDesktop,
            pause_visualizer: level >= CalmRule::PauseVisualizer,
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            theme: "nothing-dark".into(),
            hide_taskbar: true,
            autostart: false,
            window_animations: true,
            genie_effect: true,
            lock_on_start: true,
            dock: DockConfig::default(),
            top_bar: TopBarConfig::default(),
            island: IslandConfig::default(),
            widgets: WidgetsConfig::default(),
            launcher: LauncherConfig::default(),
            calm: CalmConfig::default(),
            wallpaper: WallpaperConfig::default(),
            language: "es".into(),
        }
    }
}

impl Default for DockConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            reserve_space: true,
            show_running: true,
            custom_icons: true,
            files_style: "classic".into(),
            window_previews: true,
        }
    }
}

impl Default for TopBarConfig {
    fn default() -> Self {
        Self { enabled: true, date_format: "ddd d MMM".into() }
    }
}

impl Default for IslandConfig {
    fn default() -> Self {
        Self { enabled: true, focus_minutes: 25, device_alerts: true }
    }
}

impl Default for WidgetsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            attach_to_wallpaper: false,
            items: ["calendar", "weather", "system", "signal_01"].map(String::from).to_vec(),
            weather_city: "Bogotá".into(),
            signal_mode: "face".into(),
        }
    }
}

impl Default for LauncherConfig {
    fn default() -> Self {
        Self { hotkey: "win".into(), max_results: 8 }
    }
}

impl Default for CalmConfig {
    fn default() -> Self {
        Self {
            focused: CalmRule::Keep,
            maximized: CalmRule::Keep,
            fullscreen: CalmRule::PauseDesktop,
            battery: CalmRule::PauseVisualizer,
        }
    }
}

impl AppConfig {
    /// Loads the config, writing the defaults on first run. A broken file is
    /// reported and replaced in memory by defaults (never overwritten).
    pub fn load_or_create(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(src) => match toml::from_str(&src) {
                Ok(cfg) => cfg,
                Err(e) => {
                    log::error!("{}: {e}; using defaults", path.display());
                    Self::default()
                }
            },
            Err(_) => {
                let cfg = Self::default();
                if let Err(e) = cfg.save(path) {
                    log::warn!("cannot write {}: {e}", path.display());
                }
                cfg
            }
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let src = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, src)
    }
}

/// Where everything lives. `FAMILIAR_HOME` overrides both roots (portable
/// mode / testing).
#[derive(Clone, Debug)]
pub struct Paths {
    pub data: PathBuf,
    pub cache: PathBuf,
}

impl Paths {
    pub fn resolve() -> Self {
        if let Some(home) = std::env::var_os("FAMILIAR_HOME") {
            let home = PathBuf::from(home);
            return Self { cache: home.join("cache"), data: home };
        }
        let env_dir = |var: &str| std::env::var_os(var).map(PathBuf::from);
        let data = env_dir("APPDATA")
            .or_else(|| env_dir("XDG_CONFIG_HOME"))
            .or_else(|| env_dir("HOME").map(|h| h.join(".config")))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("FamiliarCore");
        let cache = env_dir("LOCALAPPDATA")
            .or_else(|| env_dir("XDG_CACHE_HOME"))
            .or_else(|| env_dir("HOME").map(|h| h.join(".cache")))
            .map(|p| p.join("FamiliarCore"))
            .unwrap_or_else(|| data.join("cache"));
        Self { data, cache }
    }

    pub fn ensure(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.data)?;
        std::fs::create_dir_all(self.themes_dir())?;
        std::fs::create_dir_all(self.icon_dir())
    }

    pub fn config(&self) -> PathBuf {
        self.data.join("config.toml")
    }

    pub fn db(&self) -> PathBuf {
        self.data.join("familiar_state.db")
    }

    pub fn log(&self) -> PathBuf {
        self.data.join("familiar-core.log")
    }

    pub fn themes_dir(&self) -> PathBuf {
        self.data.join("themes")
    }

    pub fn icon_dir(&self) -> PathBuf {
        self.cache.join("icons")
    }

    /// Present while the native taskbar is hidden; lets the next start (or
    /// `--restore-taskbar`) undo a hard crash.
    pub fn taskbar_marker(&self) -> PathBuf {
        self.data.join("taskbar-hidden")
    }

    /// Present once the dock has been seeded with default apps.
    pub fn seeded_marker(&self) -> PathBuf {
        self.data.join("dock-seeded")
    }

    /// `ember-dark` → `<themes>/ember-dark.toml`; anything that looks like a
    /// path is used as-is.
    pub fn resolve_theme(&self, name: &str) -> PathBuf {
        let p = Path::new(name);
        if p.extension().is_some_and(|e| e == "toml") || p.components().count() > 1 {
            p.to_path_buf()
        } else {
            self.themes_dir().join(format!("{name}.toml"))
        }
    }

    /// Copies the built-in themes next to user themes so they can be edited
    /// (and hot-reloaded). Existing files are never overwritten.
    pub fn install_builtin_themes(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(self.themes_dir())?;
        for (stem, src) in familiar_theme::BUILTIN {
            let path = self.themes_dir().join(format!("{stem}.toml"));
            if !path.exists() {
                std::fs::write(path, src)?;
            }
        }
        Ok(())
    }
}

/// Stable 64-bit FNV-1a, used for cache file names.
pub fn stable_hash(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.to_lowercase().bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip() {
        let cfg = AppConfig::default();
        let src = toml::to_string_pretty(&cfg).unwrap();
        assert_eq!(toml::from_str::<AppConfig>(&src).unwrap(), cfg);
    }

    #[test]
    fn partial_file_keeps_other_defaults() {
        let cfg: AppConfig = toml::from_str("hide_taskbar = false\n[launcher]\nhotkey = \"alt+space\"\n").unwrap();
        assert!(!cfg.hide_taskbar);
        assert_eq!(cfg.launcher.hotkey, "alt+space");
        assert_eq!(cfg.launcher.max_results, 8);
        assert!(cfg.dock.enabled);
    }

    #[test]
    fn load_or_create_writes_defaults_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        assert_eq!(AppConfig::load_or_create(&path), AppConfig::default());
        std::fs::write(&path, "theme = \"mono-signal\"\n").unwrap();
        assert_eq!(AppConfig::load_or_create(&path).theme, "mono-signal");
        std::fs::write(&path, "theme = [").unwrap();
        assert_eq!(AppConfig::load_or_create(&path), AppConfig::default());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "theme = [");
    }

    #[test]
    fn theme_resolution() {
        let p = Paths { data: PathBuf::from("/d"), cache: PathBuf::from("/c") };
        assert_eq!(p.resolve_theme("ember-dark"), PathBuf::from("/d/themes/ember-dark.toml"));
        assert_eq!(p.resolve_theme("custom.toml"), PathBuf::from("custom.toml"));
        assert_eq!(p.resolve_theme("/x/y"), PathBuf::from("/x/y"));
    }

    #[test]
    fn hash_is_stable_and_case_insensitive() {
        assert_eq!(stable_hash("C:\\App.exe"), stable_hash("c:\\app.exe"));
        assert_eq!(stable_hash(""), 0xcbf2_9ce4_8422_2325);
    }

    #[test]
    fn calm_strictest_rule_wins() {
        use super::{CalmConfig, CalmInputs, CalmRule, CalmState};
        let cfg = CalmConfig::default();
        assert_eq!(cfg.evaluate(CalmInputs::default()), CalmState::default());
        // Defaults: a game hides everything; battery only pauses the visualizer.
        let game = cfg.evaluate(CalmInputs { focused: true, fullscreen: true, ..Default::default() });
        assert!(game.hide_all && game.pause_desktop && game.pause_visualizer);
        let laptop = cfg.evaluate(CalmInputs { on_battery: true, ..Default::default() });
        assert_eq!(laptop, CalmState { hide_all: false, pause_desktop: false, pause_visualizer: true });
        // A stricter rule elsewhere never hides the bar outside full screen.
        let strict = CalmConfig { maximized: CalmRule::PauseDesktop, ..cfg };
        let s = strict.evaluate(CalmInputs { focused: true, maximized: true, ..Default::default() });
        assert!(!s.hide_all && s.pause_desktop && s.pause_visualizer);
    }

    #[test]
    fn calm_rules_cycle_and_serialise() {
        use super::CalmRule;
        assert_eq!(CalmRule::Keep.next().next().next(), CalmRule::Keep);
        let cfg: super::AppConfig = toml::from_str("[calm]\nfocused = \"pause_visualizer\"").unwrap();
        assert_eq!(cfg.calm.focused, CalmRule::PauseVisualizer);
        assert_eq!(cfg.calm.fullscreen, CalmRule::PauseDesktop);
    }
}
