use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// An sRGB colour with straight alpha, written as `#RRGGBB` or `#RRGGBBAA`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// `0x00BBGGRR`, the COLORREF layout Win32/DWM expects.
    pub fn to_colorref(self) -> u32 {
        u32::from(self.r) | (u32::from(self.g) << 8) | (u32::from(self.b) << 16)
    }

    /// Relative luminance in `0.0..=1.0`; used to pick dark/light DWM mode.
    pub fn luminance(self) -> f32 {
        let lin = |c: u8| {
            let c = c as f32 / 255.0;
            if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * lin(self.r) + 0.7152 * lin(self.g) + 0.0722 * lin(self.b)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseColorError(pub String);

impl fmt::Display for ParseColorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid colour {:?}, expected #RRGGBB or #RRGGBBAA", self.0)
    }
}

impl std::error::Error for ParseColorError {}

impl FromStr for Color {
    type Err = ParseColorError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || ParseColorError(s.to_owned());
        let hex = s.trim().strip_prefix('#').ok_or_else(err)?;
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(err());
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| err());
        match hex.len() {
            6 => Ok(Color::rgba(byte(0)?, byte(2)?, byte(4)?, 255)),
            8 => Ok(Color::rgba(byte(0)?, byte(2)?, byte(4)?, byte(6)?)),
            _ => Err(err()),
        }
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.a == 255 {
            write!(f, "#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
        } else {
            write!(f, "#{:02X}{:02X}{:02X}{:02X}", self.r, self.g, self.b, self.a)
        }
    }
}

impl Serialize for Color {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_formats() {
        let c: Color = "#FF6B4A".parse().unwrap();
        assert_eq!(c, Color::rgba(0xFF, 0x6B, 0x4A, 0xFF));
        assert_eq!(c.to_string(), "#FF6B4A");
        let c: Color = "#121214cc".parse().unwrap();
        assert_eq!(c.a, 0xCC);
        assert_eq!(c.to_string(), "#121214CC");
        assert_eq!(c.to_colorref(), 0x0014_1212);
    }

    #[test]
    fn rejects_garbage() {
        for s in ["FF6B4A", "#FFF", "#GG0000", "#12345", "#+1+1+1"] {
            assert!(s.parse::<Color>().is_err(), "{s}");
        }
    }
}
