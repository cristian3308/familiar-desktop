//! Generated themes: an accent picked from the wallpaper, and "Surprise me"
//! random-but-harmonious palettes. Pure CPU, no image decoding here: the
//! caller passes small RGBA thumbnails.

use crate::{BlurType, Color, Colors, Meta, Metrics, Theme};

/// Hue in degrees (0..360), saturation and lightness in 0..1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hsl {
    pub h: f32,
    pub s: f32,
    pub l: f32,
}

impl Hsl {
    pub fn from_color(c: Color) -> Self {
        let (r, g, b) = (c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let l = (max + min) / 2.0;
        let d = max - min;
        if d < 1e-6 {
            return Self { h: 0.0, s: 0.0, l };
        }
        let s = d / (1.0 - (2.0 * l - 1.0).abs());
        let h = if max == r {
            60.0 * ((g - b) / d).rem_euclid(6.0)
        } else if max == g {
            60.0 * ((b - r) / d + 2.0)
        } else {
            60.0 * ((r - g) / d + 4.0)
        };
        Self { h, s: s.clamp(0.0, 1.0), l }
    }

    pub fn to_color(self, a: u8) -> Color {
        let h = self.h.rem_euclid(360.0);
        let (s, l) = (self.s.clamp(0.0, 1.0), self.l.clamp(0.0, 1.0));
        let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
        let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
        let m = l - c / 2.0;
        let (r, g, b) = match (h / 60.0) as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let to = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
        Color::rgba(to(r), to(g), to(b), a)
    }
}

/// What a wallpaper thumbnail looks like, for building a theme around it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageMood {
    /// The most prominent vivid colour (falls back to a neutral accent for
    /// grey images).
    pub accent: Color,
    /// Average luminance, 0 (black) .. 1 (white).
    pub brightness: f32,
}

/// Analyses straight-alpha RGBA pixels: vivid colours are binned by hue
/// (weighted by saturation, so a small red sign beats a big grey sky) and
/// the strongest bin's average becomes the accent.
pub fn analyse(rgba: &[u8]) -> ImageMood {
    const BINS: usize = 24;
    let mut weight = [0f32; BINS];
    let mut sum = [[0f32; 3]; BINS];
    let mut lum = 0f32;
    let mut count = 0f32;
    for px in rgba.as_chunks::<4>().0 {
        if px[3] < 128 {
            continue;
        }
        let c = Color::rgba(px[0], px[1], px[2], 255);
        lum += c.luminance();
        count += 1.0;
        let hsl = Hsl::from_color(c);
        // Ignore greys and near-black / near-white pixels.
        if hsl.s < 0.25 || hsl.l < 0.12 || hsl.l > 0.92 {
            continue;
        }
        let bin = ((hsl.h / 360.0 * BINS as f32) as usize).min(BINS - 1);
        let w = hsl.s * (1.0 - (hsl.l - 0.5).abs());
        weight[bin] += w;
        sum[bin][0] += px[0] as f32 * w;
        sum[bin][1] += px[1] as f32 * w;
        sum[bin][2] += px[2] as f32 * w;
    }
    let brightness = if count > 0.0 { lum / count } else { 0.0 };
    let best = (0..BINS).max_by(|a, b| weight[*a].total_cmp(&weight[*b])).unwrap_or(0);
    let accent = if weight[best] > count * 0.01 && weight[best] > 0.0 {
        let w = weight[best];
        let raw = Color::rgba((sum[best][0] / w) as u8, (sum[best][1] / w) as u8, (sum[best][2] / w) as u8, 255);
        // Make it usable as an accent: vivid and mid-bright.
        let mut hsl = Hsl::from_color(raw);
        hsl.s = hsl.s.max(0.65);
        hsl.l = hsl.l.clamp(0.5, 0.62);
        hsl.to_color(255)
    } else {
        Color::rgba(0xFF, 0x2B, 0x2B, 255)
    };
    ImageMood { accent, brightness }
}

/// A full theme around `accent`: tinted near-black (or near-white) glass
/// whose hue is `surface_hue` (the accent's own hue for a calm theme).
pub fn theme_from_accent(name: &str, accent: Color, surface_hue: f32, dark: bool) -> Theme {
    let tint = |s: f32, l: f32, a: u8| Hsl { h: surface_hue, s, l }.to_color(a);
    let accent_glow = Color::rgba(accent.r, accent.g, accent.b, 0x40);
    let colors = if dark {
        Colors {
            background_tint: tint(0.35, 0.08, 0xE6),
            accent,
            text_primary: tint(0.25, 0.96, 0xFF),
            text_secondary: tint(0.15, 0.66, 0xFF),
            border_glow: accent_glow,
            island_background: tint(0.4, 0.03, 0xFF),
            dot_off: Color::rgba(0xFF, 0xFF, 0xFF, 0x14),
            bar_tint: tint(0.35, 0.08, 0x4D),
        }
    } else {
        Colors {
            background_tint: tint(0.3, 0.95, 0xE6),
            accent,
            text_primary: tint(0.2, 0.14, 0xFF),
            text_secondary: tint(0.1, 0.45, 0xFF),
            border_glow: Color::rgba(0, 0, 0, 0x1A),
            island_background: tint(0.2, 0.06, 0xFF),
            dot_off: Color::rgba(0, 0, 0, 0x1A),
            bar_tint: tint(0.3, 0.97, 0x66),
        }
    };
    Theme {
        meta: Meta { name: name.into(), author: "Familiar Core".into() },
        colors,
        metrics: Metrics {
            dock_icon_size: 44,
            dock_magnification: 1.5,
            corner_radius: 18,
            top_bar_height: 32,
            blur_type: if dark { BlurType::Acrylic } else { BlurType::Mica },
        },
        cursor: Default::default(),
    }
}

/// Theme generated from a wallpaper thumbnail.
pub fn from_wallpaper(rgba: &[u8]) -> Theme {
    let mood = analyse(rgba);
    let hue = Hsl::from_color(mood.accent).h;
    // Bright wallpapers get light glass, like macOS does.
    theme_from_accent("Wallpaper", mood.accent, hue, mood.brightness < 0.55)
}

/// A random but harmonious theme for "Surprise me". The same seed always
/// gives the same theme.
pub fn surprise(seed: u64) -> Theme {
    let mut x = seed ^ 0x9E37_79B9_7F4A_7C15;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        (x % 10_000) as f32 / 10_000.0
    };
    let hue = next() * 360.0;
    // Surface hue: same family, complementary or split-complementary.
    let offset = [0.0, 180.0, 150.0, 210.0, 30.0][(next() * 5.0) as usize % 5];
    let accent = Hsl { h: hue, s: 0.7 + next() * 0.25, l: 0.55 + next() * 0.07 }.to_color(255);
    let dark = next() < 0.75;
    let mut t = theme_from_accent("Surprise", accent, hue + offset, dark);
    t.metrics.corner_radius = [12, 16, 18, 22, 26][(next() * 5.0) as usize % 5];
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(c: Color, n: usize) -> Vec<u8> {
        std::iter::repeat_n([c.r, c.g, c.b, 255], n).flatten().collect()
    }

    #[test]
    fn hsl_round_trips() {
        for c in [Color::rgba(255, 43, 43, 255), Color::rgba(47, 123, 255, 255), Color::rgba(123, 211, 137, 255)] {
            let back = Hsl::from_color(c).to_color(255);
            assert!((back.r as i32 - c.r as i32).abs() <= 1, "{c:?} -> {back:?}");
            assert!((back.g as i32 - c.g as i32).abs() <= 1, "{c:?} -> {back:?}");
            assert!((back.b as i32 - c.b as i32).abs() <= 1, "{c:?} -> {back:?}");
        }
    }

    #[test]
    fn vivid_colour_beats_grey() {
        // Mostly grey sky with some orange.
        let mut px = solid(Color::rgba(128, 128, 130, 255), 900);
        px.extend(solid(Color::rgba(255, 120, 20, 255), 100));
        let mood = analyse(&px);
        let h = Hsl::from_color(mood.accent).h;
        assert!((15.0..40.0).contains(&h), "hue {h}");
    }

    #[test]
    fn grey_images_get_a_default_accent_and_brightness() {
        let dark = analyse(&solid(Color::rgba(20, 20, 20, 255), 100));
        assert_eq!(dark.accent, Color::rgba(0xFF, 0x2B, 0x2B, 255));
        assert!(dark.brightness < 0.05);
        let light = analyse(&solid(Color::rgba(240, 240, 240, 255), 100));
        assert!(light.brightness > 0.8);
    }

    #[test]
    fn wallpaper_theme_follows_brightness() {
        let night = from_wallpaper(&solid(Color::rgba(10, 20, 60, 255), 100));
        assert!(night.colors.text_primary.luminance() > 0.7);
        let day = from_wallpaper(&solid(Color::rgba(230, 240, 255, 255), 100));
        assert!(day.colors.text_primary.luminance() < 0.1);
    }

    #[test]
    fn surprise_is_deterministic_and_valid() {
        assert_eq!(surprise(42), surprise(42));
        assert_ne!(surprise(1).colors.accent, surprise(2).colors.accent);
        for seed in 0..50 {
            let t = surprise(seed);
            let back = Theme::from_toml(&t.to_toml().unwrap()).unwrap();
            assert_eq!(back, t);
            // Text always contrasts with the surface.
            let contrast = (t.colors.text_primary.luminance() - t.colors.background_tint.luminance()).abs();
            assert!(contrast > 0.5, "seed {seed}: contrast {contrast}");
        }
    }
}
