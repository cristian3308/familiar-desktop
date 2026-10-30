//! SIGNAL/01: the dot-matrix widget's five modes.
//!
//! Every mode is a pure function of the band levels, the time and a little
//! [`SignalState`], so frames are cheap and reproducible in tests. All modes
//! keep moving while silent; audio only adds energy.

use std::f32::consts::TAU;

use crate::Matrix;
use crate::font::{GLYPH_H, text_width};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignalMode {
    /// A face that blinks and sings along.
    Face,
    /// A radial pulse breathing with the bass.
    Pulse,
    /// Concentric rings spreading from the centre.
    Ripple,
    /// A rotating 3D torus.
    Torus,
    /// HH:MM with a seconds bar.
    Clock,
}

impl SignalMode {
    pub const ALL: [SignalMode; 5] =
        [SignalMode::Face, SignalMode::Pulse, SignalMode::Ripple, SignalMode::Torus, SignalMode::Clock];

    /// Stable id for `config.toml`.
    pub fn id(self) -> &'static str {
        match self {
            SignalMode::Face => "face",
            SignalMode::Pulse => "pulse",
            SignalMode::Ripple => "ripple",
            SignalMode::Torus => "torus",
            SignalMode::Clock => "clock",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.id().eq_ignore_ascii_case(id))
    }

    /// Label shown on the widget.
    pub fn name(self) -> &'static str {
        match self {
            SignalMode::Face => "FACE",
            SignalMode::Pulse => "PULSE",
            SignalMode::Ripple => "RIPPLE",
            SignalMode::Torus => "TORUS",
            SignalMode::Clock => "CLOCK",
        }
    }

    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|m| *m == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}

/// Time and wall clock for one frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct Frame {
    /// Seconds since the widget started; drives idle motion.
    pub t: f32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

#[derive(Clone, Debug, Default)]
pub struct SignalState {
    /// Smoothed overall energy, so motion does not flicker with the audio.
    energy: f32,
}

fn avg(bands: &[f32], r: std::ops::Range<usize>) -> f32 {
    let r = r.start.min(bands.len())..r.end.min(bands.len());
    if r.is_empty() { 0.0 } else { bands[r.clone()].iter().map(|b| b.clamp(0.0, 1.0)).sum::<f32>() / r.len() as f32 }
}

pub fn draw(m: &mut Matrix, mode: SignalMode, bands: &[f32], f: Frame, st: &mut SignalState) {
    m.clear();
    let target = avg(bands, 0..bands.len());
    st.energy += (target - st.energy) * 0.35;
    match mode {
        SignalMode::Face => face(m, bands, f),
        SignalMode::Pulse => pulse(m, avg(bands, 0..8), f, st.energy),
        SignalMode::Ripple => ripple(m, f, st.energy),
        SignalMode::Torus => torus(m, f, st.energy),
        SignalMode::Clock => clock(m, f),
    }
}

/// Eyes are closed for a short moment every few seconds.
fn blinking(t: f32) -> bool {
    let period = 4.2;
    t.rem_euclid(period) > period - 0.16
}

fn face(m: &mut Matrix, bands: &[f32], f: Frame) {
    m.draw_signal_face(bands);
    if blinking(f.t) {
        // Close the eyes (upper half): keep only their bottom row.
        let cols = m.cols;
        for x in 0..cols {
            if let Some(low) = (0..m.rows / 2).rev().find(|&y| m.get(x, y) > 0.0) {
                for y in 0..low {
                    m.dots[y * cols + x] = 0.0;
                }
            }
        }
    }
}

fn centre(m: &Matrix) -> (f32, f32) {
    (m.cols as f32 / 2.0 - 0.5, m.rows as f32 / 2.0 - 0.5)
}

fn pulse(m: &mut Matrix, bass: f32, f: Frame, energy: f32) {
    let (cx, cy) = centre(m);
    let max_r = m.rows as f32 * 0.5;
    let breathe = ((f.t * 2.4).sin() * 0.5 + 0.5) * 0.35;
    let r = max_r * (0.35 + breathe + bass * 0.45).min(1.0);
    let core = 1.0 + energy * 3.0;
    for y in 0..m.rows {
        for x in 0..m.cols {
            let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
            let ring = (1.0 - (d - r).abs() / 1.3).max(0.0);
            let glow = if d <= core { 1.0 } else { 0.0 };
            m.set(x as i32, y as i32, ring.max(glow));
        }
    }
}

fn ripple(m: &mut Matrix, f: Frame, energy: f32) {
    let (cx, cy) = centre(m);
    let max_r = (cx * cx + cy * cy).sqrt();
    let spacing = 4.0;
    let speed = 5.0 + energy * 10.0;
    for y in 0..m.rows {
        for x in 0..m.cols {
            let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
            // Distance to the nearest ring travelling outwards.
            let phase = (d - f.t * speed).rem_euclid(spacing);
            let near = phase.min(spacing - phase);
            let k = (1.0 - near / 0.9).max(0.0) * (1.0 - d / (max_r + 1.0)).max(0.15);
            m.set(x as i32, y as i32, k * (0.6 + energy * 0.8));
        }
    }
    m.set(cx.round() as i32, cy.round() as i32, 1.0);
}

fn torus(m: &mut Matrix, f: Frame, energy: f32) {
    let (cx, cy) = centre(m);
    let (big, small) = (1.0, 0.45);
    let (a, b) = (f.t * 0.9, f.t * 0.55);
    let (sa, ca, sb, cb) = (a.sin(), a.cos(), b.sin(), b.cos());
    let scale = m.rows as f32 * (0.62 + energy * 0.12);
    let mut zbuf = vec![f32::MIN; m.cols * m.rows];
    let (steps_u, steps_v) = (90, 36);
    for i in 0..steps_u {
        let u = i as f32 / steps_u as f32 * TAU;
        let (su, cu) = (u.sin(), u.cos());
        for j in 0..steps_v {
            let v = j as f32 / steps_v as f32 * TAU;
            let (sv, cv) = (v.sin(), v.cos());
            let ring = big + small * cv;
            // Point on the torus, rotated about X (a) then Z (b).
            let (x0, y0, z0) = (ring * cu, ring * su, small * sv);
            let (y1, z1) = (y0 * ca - z0 * sa, y0 * sa + z0 * ca);
            let (x2, y2) = (x0 * cb - y1 * sb, x0 * sb + y1 * cb);
            let depth = 1.0 / (z1 + 3.2);
            let px = (cx + x2 * scale * depth * 1.6).round() as i32;
            let py = (cy + y2 * scale * depth * 1.6).round() as i32;
            if px < 0 || py < 0 || px as usize >= m.cols || py as usize >= m.rows {
                continue;
            }
            let idx = py as usize * m.cols + px as usize;
            if depth > zbuf[idx] {
                zbuf[idx] = depth;
                // Nearer points glow brighter.
                let lum = ((depth - 0.22) / 0.25).clamp(0.25, 1.0);
                m.dots[idx] = lum;
            }
        }
    }
}

fn clock(m: &mut Matrix, f: Frame) {
    let text = format!("{:02}:{:02}", f.hour, f.minute);
    let w = text_width(&text) as i32;
    let x = (m.cols as i32 - w) / 2;
    let y = ((m.rows as i32 - GLYPH_H as i32) / 2 - 1).max(0);
    // The colon blinks with the seconds.
    let shown = if f.second.is_multiple_of(2) { text.clone() } else { text.replace(':', " ") };
    m.draw_text(&shown, x, y, 1.0);
    // Seconds bar along the bottom row.
    let bar_y = m.rows as i32 - 2;
    let filled = (f.second as usize * m.cols).div_ceil(60);
    for x in 0..m.cols {
        m.set(x as i32, bar_y, if x < filled { 1.0 } else { 0.18 });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(t: f32) -> Frame {
        Frame { t, hour: 12, minute: 34, second: 30 }
    }

    #[test]
    fn every_mode_draws_while_silent() {
        for mode in SignalMode::ALL {
            let mut m = Matrix::new(32, 16);
            draw(&mut m, mode, &[0.0; 64], frame(1.0), &mut SignalState::default());
            assert!(m.lit() > 8, "{mode:?} drew {} dots", m.lit());
        }
    }

    #[test]
    fn animated_modes_change_over_time() {
        for mode in [SignalMode::Pulse, SignalMode::Ripple, SignalMode::Torus] {
            let mut a = Matrix::new(32, 16);
            let mut b = Matrix::new(32, 16);
            draw(&mut a, mode, &[0.0; 64], frame(0.0), &mut SignalState::default());
            draw(&mut b, mode, &[0.0; 64], frame(0.7), &mut SignalState::default());
            assert_ne!(a, b, "{mode:?} is static");
        }
    }

    #[test]
    fn face_blinks() {
        let mut open = Matrix::new(32, 16);
        let mut shut = Matrix::new(32, 16);
        draw(&mut open, SignalMode::Face, &[0.0; 64], frame(1.0), &mut SignalState::default());
        draw(&mut shut, SignalMode::Face, &[0.0; 64], frame(4.15), &mut SignalState::default());
        assert!(shut.lit() < open.lit());
    }

    #[test]
    fn pulse_grows_with_bass() {
        let lit = |level: f32| {
            let mut m = Matrix::new(32, 16);
            let mut st = SignalState::default();
            draw(&mut m, SignalMode::Pulse, &[level; 64], frame(0.0), &mut st);
            m.lit()
        };
        assert!(lit(1.0) > lit(0.0));
    }

    #[test]
    fn clock_shows_the_time() {
        let mut m = Matrix::new(32, 16);
        draw(&mut m, SignalMode::Clock, &[], frame(0.0), &mut SignalState::default());
        let mut expected = Matrix::new(32, 16);
        expected.draw_text("12:34", (32 - text_width("12:34") as i32) / 2, 3, 1.0);
        for (i, d) in expected.dots.iter().enumerate() {
            if *d > 0.0 {
                assert!(m.dots[i] > 0.0, "missing digit dot {i}");
            }
        }
    }

    #[test]
    fn ids_round_trip() {
        for mode in SignalMode::ALL {
            assert_eq!(SignalMode::from_id(mode.id()), Some(mode));
        }
        assert_eq!(SignalMode::Clock.next(), SignalMode::Face);
    }
}
