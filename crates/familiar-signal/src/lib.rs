//! Audio-reactive building blocks.
//!
//! * [`spectrum`] turns mono PCM into 64 log-spaced, smoothed band levels.
//! * [`matrix`] builds dot-matrix frames (bars, text, the SIGNAL/01 face).
//! * [`raster`] draws a frame into an RGBA buffer the UI uploads as one
//!   texture, which is far cheaper than animating hundreds of UI elements.

pub mod font;
pub mod matrix;
pub mod raster;
pub mod scenes;
pub mod spectrum;

pub use matrix::Matrix;
pub use raster::{DotStyle, Rgba};
pub use spectrum::SpectrumAnalyzer;
