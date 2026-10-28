//! Theme model, parsing, share codes and hot reload.
//!
//! A theme is a small TOML document (see `themes/ember-dark.toml`). Themes
//! can be shared as a single line of text: `FAM1:` followed by the URL-safe
//! Base64 of the TOML.

mod color;
#[cfg(feature = "watch")]
mod watch;

use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};

pub use color::{Color, ParseColorError};
#[cfg(feature = "watch")]
pub use watch::ThemeWatcher;

pub const SHARE_PREFIX: &str = "FAM1:";

/// Themes shipped inside the binary: `(file stem, TOML source)`.
pub const BUILTIN: &[(&str, &str)] = &[
    ("ember-dark", include_str!("../themes/ember-dark.toml")),
    ("glacier-light", include_str!("../themes/glacier-light.toml")),
    ("mono-signal", include_str!("../themes/mono-signal.toml")),
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Theme {
    pub meta: Meta,
    pub colors: Colors,
    pub metrics: Metrics,
    pub cursor: CursorFx,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Meta {
    pub name: String,
    pub author: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Colors {
    pub background_tint: Color,
    pub accent: Color,
    pub text_primary: Color,
    pub text_secondary: Color,
    pub border_glow: Color,
    pub island_background: Color,
    pub dot_off: Color,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Metrics {
    pub dock_icon_size: u32,
    pub dock_magnification: f32,
    pub corner_radius: u32,
    pub top_bar_height: u32,
    pub blur_type: BlurType,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlurType {
    None,
    Mica,
    #[default]
    Acrylic,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CursorFx {
    /// Replace the system cursors with the `.cur`/`.ani` files in
    /// `scheme_dir` (relative to the theme file).
    pub custom_scheme: bool,
    pub scheme_dir: Option<String>,
    pub ripple_on_click: bool,
    pub shake_to_find: bool,
}

/// The default theme is Ember Dark; `#[serde(default)]` relies on this not
/// going through the parser (that would recurse).
impl Default for Theme {
    fn default() -> Self {
        Self {
            meta: Meta { name: "Ember Dark".into(), author: "Familiar Core".into() },
            colors: Colors::default(),
            metrics: Metrics::default(),
            cursor: CursorFx::default(),
        }
    }
}

impl Default for Meta {
    fn default() -> Self {
        Self { name: "Untitled".into(), author: "Unknown".into() }
    }
}

impl Default for Colors {
    fn default() -> Self {
        let c = |s: &str| s.parse::<Color>().expect("valid default colour");
        Self {
            background_tint: c("#121214CC"),
            accent: c("#FF6B4A"),
            text_primary: c("#F5F5F7"),
            text_secondary: c("#A1A1A6"),
            border_glow: c("#FFFFFF22"),
            island_background: c("#000000F2"),
            dot_off: c("#FFFFFF14"),
        }
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            dock_icon_size: 48,
            dock_magnification: 1.45,
            corner_radius: 18,
            top_bar_height: 32,
            blur_type: BlurType::Acrylic,
        }
    }
}

impl Default for CursorFx {
    fn default() -> Self {
        Self { custom_scheme: false, scheme_dir: None, ripple_on_click: true, shake_to_find: true }
    }
}

#[derive(Debug)]
pub enum ThemeError {
    Io(std::io::Error),
    Parse(toml::de::Error),
    Serialize(toml::ser::Error),
    BadShareCode(&'static str),
    UnknownBuiltin(String),
}

impl std::fmt::Display for ThemeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "cannot read theme: {e}"),
            Self::Parse(e) => write!(f, "invalid theme: {e}"),
            Self::Serialize(e) => write!(f, "cannot encode theme: {e}"),
            Self::BadShareCode(why) => write!(f, "invalid share code: {why}"),
            Self::UnknownBuiltin(n) => write!(f, "no built-in theme named {n:?}"),
        }
    }
}

impl std::error::Error for ThemeError {}

impl Theme {
    pub fn from_toml(src: &str) -> Result<Self, ThemeError> {
        let mut t: Theme = toml::from_str(src).map_err(ThemeError::Parse)?;
        t.sanitize();
        Ok(t)
    }

    pub fn load(path: &Path) -> Result<Self, ThemeError> {
        Self::from_toml(&std::fs::read_to_string(path).map_err(ThemeError::Io)?)
    }

    pub fn builtin(stem: &str) -> Result<Self, ThemeError> {
        let (_, src) =
            BUILTIN.iter().find(|(s, _)| *s == stem).ok_or_else(|| ThemeError::UnknownBuiltin(stem.into()))?;
        Self::from_toml(src)
    }

    pub fn to_toml(&self) -> Result<String, ThemeError> {
        toml::to_string_pretty(self).map_err(ThemeError::Serialize)
    }

    pub fn to_share_code(&self) -> Result<String, ThemeError> {
        let compact = toml::to_string(self).map_err(ThemeError::Serialize)?;
        Ok(format!("{SHARE_PREFIX}{}", URL_SAFE_NO_PAD.encode(compact)))
    }

    pub fn from_share_code(code: &str) -> Result<Self, ThemeError> {
        let body = code.trim().strip_prefix(SHARE_PREFIX).ok_or(ThemeError::BadShareCode("missing FAM1: prefix"))?;
        let bytes =
            URL_SAFE_NO_PAD.decode(body.trim_end_matches('=')).map_err(|_| ThemeError::BadShareCode("not base64"))?;
        let src = String::from_utf8(bytes).map_err(|_| ThemeError::BadShareCode("not UTF-8"))?;
        Self::from_toml(&src)
    }

    /// Clamps metrics to ranges the shell can render sensibly, so a typo in
    /// a hand-edited file cannot produce a 4000 px dock.
    pub fn sanitize(&mut self) {
        let m = &mut self.metrics;
        m.dock_icon_size = m.dock_icon_size.clamp(24, 128);
        m.dock_magnification =
            if m.dock_magnification.is_finite() { m.dock_magnification.clamp(1.0, 2.5) } else { 1.0 };
        m.corner_radius = m.corner_radius.clamp(0, 40);
        m.top_bar_height = m.top_bar_height.clamp(22, 48);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_parse() {
        for (stem, _) in BUILTIN {
            Theme::builtin(stem).unwrap();
        }
        assert_eq!(Theme::default(), Theme::builtin("ember-dark").unwrap());
    }

    #[test]
    fn share_code_round_trips() {
        let t = Theme::builtin("glacier-light").unwrap();
        let code = t.to_share_code().unwrap();
        assert!(code.starts_with(SHARE_PREFIX));
        assert!(!code.contains('\n'));
        assert_eq!(Theme::from_share_code(&code).unwrap(), t);
    }

    #[test]
    fn bad_share_codes_are_rejected() {
        assert!(Theme::from_share_code("hello").is_err());
        assert!(Theme::from_share_code("FAM1:!!!").is_err());
    }

    #[test]
    fn partial_themes_fill_defaults_and_get_clamped() {
        let t = Theme::from_toml("[metrics]\ndock_icon_size = 4000\nblur_type = \"Mica\"").unwrap();
        assert_eq!(t.metrics.dock_icon_size, 128);
        assert_eq!(t.metrics.blur_type, BlurType::Mica);
        assert_eq!(t.colors, Colors::default());
    }

    #[test]
    fn toml_round_trips() {
        let t = Theme::default();
        assert_eq!(Theme::from_toml(&t.to_toml().unwrap()).unwrap(), t);
    }
}
