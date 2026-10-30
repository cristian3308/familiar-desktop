//! Music visualizer scenes drawn into a dot matrix.
//!
//! Every scene maps the 64 log-spaced band levels (0..=1) onto the frame.
//! Scenes with memory (wave history, phosphor glow, falling peaks, sparks)
//! keep it in [`SceneState`], which is reset when the scene changes.

use std::f32::consts::TAU;

use crate::Matrix;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scene {
    /// Spectrum lines receding towards a horizon, newest in front.
    Horizon,
    /// A cloud of particles whose spread follows each band.
    Nebula,
    /// Circular spectrum orbiting the centre, with a satellite dot.
    Orbit,
    /// Oscilloscope trace whose glow decays like phosphor.
    Phosphor,
    /// Topographic contour lines of a band-driven height field.
    Contour,
    /// Rising bars with gravity peaks and sparks.
    Ember,
}

impl Scene {
    pub const ALL: [Scene; 6] =
        [Scene::Horizon, Scene::Nebula, Scene::Orbit, Scene::Phosphor, Scene::Contour, Scene::Ember];

    pub fn name(self) -> &'static str {
        match self {
            Scene::Horizon => "HORIZON",
            Scene::Nebula => "NEBULA",
            Scene::Orbit => "ORBIT",
            Scene::Phosphor => "PHOSPHOR",
            Scene::Contour => "CONTOUR",
            Scene::Ember => "EMBER",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SceneState {
    /// Advances every frame; animates idle motion.
    tick: u32,
    /// Recent spectra, newest first (Horizon).
    history: Vec<Vec<f32>>,
    /// Persistent glow buffer (Phosphor).
    glow: Vec<f32>,
    /// Peak height and fall speed per column (Ember).
    peaks: Vec<(f32, f32)>,
    /// Sparks: x, y, life (Ember).
    sparks: Vec<(f32, f32, f32)>,
    seed: u32,
}

impl SceneState {
    /// Small xorshift: deterministic, no dependency.
    fn rand(&mut self) -> f32 {
        if self.seed == 0 {
            self.seed = 0x9E37_79B9;
        }
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed % 10_000) as f32 / 10_000.0
    }
}

/// Band level for column `x` of `cols`, resampled from `bands`.
fn level(bands: &[f32], x: usize, cols: usize) -> f32 {
    if bands.is_empty() {
        return 0.0;
    }
    let i = x * bands.len() / cols.max(1);
    bands[i.min(bands.len() - 1)].clamp(0.0, 1.0)
}

fn bass(bands: &[f32]) -> f32 {
    let n = (bands.len() / 8).max(1);
    bands.iter().take(n).map(|b| b.clamp(0.0, 1.0)).sum::<f32>() / n as f32
}

/// Stable pseudo-random value in 0..1 for particle `i`.
fn hash(i: u32) -> f32 {
    let mut x = i.wrapping_mul(0x2C1B_3C6D) ^ 0x297A_2D39;
    x ^= x >> 15;
    x = x.wrapping_mul(0x8524_9CB5);
    x ^= x >> 13;
    (x % 10_000) as f32 / 10_000.0
}

fn line(m: &mut Matrix, (x0, y0): (i32, i32), (x1, y1): (i32, i32), v: f32) {
    let steps = (x1 - x0).abs().max((y1 - y0).abs()).max(1);
    for s in 0..=steps {
        let k = s as f32 / steps as f32;
        let x = x0 as f32 + (x1 - x0) as f32 * k;
        let y = y0 as f32 + (y1 - y0) as f32 * k;
        m.set(x.round() as i32, y.round() as i32, v);
    }
}

pub fn draw(m: &mut Matrix, scene: Scene, bands: &[f32], st: &mut SceneState) {
    m.clear();
    st.tick = st.tick.wrapping_add(1);
    match scene {
        Scene::Horizon => horizon(m, bands, st),
        Scene::Nebula => nebula(m, bands, st),
        Scene::Orbit => orbit(m, bands, st),
        Scene::Phosphor => phosphor(m, bands, st),
        Scene::Contour => contour(m, bands, st),
        Scene::Ember => ember(m, bands, st),
    }
}

fn horizon(m: &mut Matrix, bands: &[f32], st: &mut SceneState) {
    const DEPTH: usize = 7;
    let (cols, rows) = (m.cols, m.rows);
    let idle: Vec<f32> =
        (0..cols).map(|x| ((x as f32 * 0.3 + st.tick as f32 * 0.08).sin() * 0.5 + 0.5) * 0.12).collect();
    let now: Vec<f32> = (0..cols).map(|x| level(bands, x, cols).max(idle[x])).collect();
    st.history.insert(0, now);
    st.history.truncate(DEPTH);
    let horizon_y = rows as f32 * 0.22;
    // Oldest (far, small, dim) first so nearer lines draw over them.
    for (d, spectrum) in st.history.iter().enumerate().rev() {
        let z = d as f32 / DEPTH as f32;
        let base = rows as f32 - 1.0 - (rows as f32 - 1.0 - horizon_y) * z;
        let width = 1.0 - z * 0.55;
        let amp = (rows as f32 * 0.55) * (1.0 - z * 0.7);
        let bright = 1.0 - z * 0.8;
        let mut prev = None;
        for (x, v) in spectrum.iter().enumerate() {
            let sx = cols as f32 / 2.0 + (x as f32 - cols as f32 / 2.0) * width;
            let p = (sx.round() as i32, (base - v * amp).round() as i32);
            if let Some(q) = prev {
                line(m, q, p, bright);
            }
            prev = Some(p);
        }
    }
}

fn nebula(m: &mut Matrix, bands: &[f32], st: &mut SceneState) {
    let (cx, cy) = (m.cols as f32 / 2.0 - 0.5, m.rows as f32 / 2.0 - 0.5);
    let spin = st.tick as f32 * 0.012;
    let energy = bass(bands);
    for i in 0..220u32 {
        let band = (hash(i) * 63.0) as usize;
        let v = bands.get(band).copied().unwrap_or(0.0).clamp(0.0, 1.0);
        let ang = hash(i ^ 0xA5A5) * TAU + spin * (0.5 + hash(i ^ 0x1234));
        let r = (0.15 + hash(i ^ 0x77) * 0.5 + v * 0.5 + energy * 0.15) * m.rows as f32 * 0.55;
        // Elliptical: the matrix is wider than tall.
        let x = cx + ang.cos() * r * (m.cols as f32 / m.rows as f32) * 0.8;
        let y = cy + ang.sin() * r;
        let twinkle = ((st.tick as f32 * 0.2 + i as f32).sin() * 0.5 + 0.5) * 0.3;
        m.set(x.round() as i32, y.round() as i32, 0.35 + v * 0.65 - twinkle * (1.0 - v));
    }
    m.set(cx.round() as i32, cy.round() as i32, 1.0);
}

fn orbit(m: &mut Matrix, bands: &[f32], st: &mut SceneState) {
    let (cx, cy) = (m.cols as f32 / 2.0 - 0.5, m.rows as f32 / 2.0 - 0.5);
    let rows = m.rows as f32;
    let r0 = rows * 0.2;
    let spokes = 48;
    let rot = st.tick as f32 * 0.02;
    for s in 0..spokes {
        let ang = s as f32 / spokes as f32 * TAU + rot;
        let v = level(bands, s, spokes);
        let len = r0 + v * rows * 0.3;
        let mut r = r0;
        while r <= len {
            m.set((cx + ang.cos() * r).round() as i32, (cy + ang.sin() * r).round() as i32, 1.0);
            r += 0.7;
        }
        m.set((cx + ang.cos() * r0).round() as i32, (cy + ang.sin() * r0).round() as i32, 0.4);
    }
    // A satellite on a wide orbit, faster with the bass.
    let sat = rot * (3.0 + bass(bands) * 4.0);
    let (ox, oy) = (rows * 0.75, rows * 0.45);
    for k in 0..3 {
        let a = sat - k as f32 * 0.12;
        m.set((cx + a.cos() * ox).round() as i32, (cy + a.sin() * oy).round() as i32, 1.0 - k as f32 * 0.3);
    }
}

fn phosphor(m: &mut Matrix, bands: &[f32], st: &mut SceneState) {
    let (cols, rows) = (m.cols, m.rows);
    st.glow.resize(cols * rows, 0.0);
    for g in st.glow.iter_mut() {
        *g *= 0.72;
    }
    let h = rows as i32;
    let mut prev: Option<(i32, i32)> = None;
    for x in 0..cols {
        let a = level(bands, x, cols);
        let carrier = (x as f32 * 0.45 + st.tick as f32 * 0.22).sin();
        let y = (rows as f32 / 2.0 - carrier * (1.5 + a * rows as f32 * 0.42)).round() as i32;
        let p = (x as i32, y.clamp(0, h - 1));
        let (x0, y0) = prev.unwrap_or(p);
        let steps = (p.1 - y0).abs().max(1);
        for s in 0..=steps {
            let yy = y0 + (p.1 - y0) * s / steps;
            let xx = if s * 2 < steps { x0 } else { p.0 };
            st.glow[yy as usize * cols + xx as usize] = 1.0;
        }
        prev = Some(p);
    }
    m.dots.copy_from_slice(&st.glow);
}

fn contour(m: &mut Matrix, bands: &[f32], st: &mut SceneState) {
    let (cols, rows) = (m.cols, m.rows);
    let t = st.tick as f32 * 0.04;
    for y in 0..rows {
        let fy = y as f32 / rows as f32;
        for x in 0..cols {
            let v = level(bands, x, cols);
            // Hills where the band is loud, centred vertically, plus a slow drift.
            let hill = v * (-((fy - 0.5) * 3.0).powi(2)).exp();
            let drift = ((x as f32 * 0.18 + t).sin() + (fy * 5.0 - t * 0.7).cos()) * 0.12;
            let z = (hill * 1.6 + drift + 0.3) * 5.0;
            let d = z.fract();
            if d < 0.16 || d > 0.92 {
                m.set(x as i32, y as i32, 0.45 + (z / 5.0).clamp(0.0, 1.0) * 0.55);
            }
        }
    }
}

fn ember(m: &mut Matrix, bands: &[f32], st: &mut SceneState) {
    let (cols, rows) = (m.cols, m.rows);
    let h = rows as i32;
    st.peaks.resize(cols, (0.0, 0.0));
    for x in 0..cols {
        let v = level(bands, x, cols) * rows as f32;
        for y in 0..v as i32 {
            // Hotter (brighter) towards the top of each bar.
            m.set(x as i32, h - 1 - y, 0.3 + 0.5 * y as f32 / v.max(1.0));
        }
        let (peak, speed) = st.peaks[x];
        st.peaks[x] = if v >= peak {
            (v, 0.0)
        } else {
            // Gravity: the peak falls faster and faster.
            let s = speed + 0.045;
            ((peak - s).max(0.0), s)
        };
        let p = st.peaks[x].0;
        if p > 0.5 {
            m.set(x as i32, h - 1 - p as i32, 1.0);
        }
        // Loud bars throw sparks from their tops.
        if v > rows as f32 * 0.45 && st.rand() < 0.18 {
            st.sparks.push((x as f32, rows as f32 - v, 1.0));
        }
    }
    let mut sparks = std::mem::take(&mut st.sparks);
    for s in sparks.iter_mut() {
        let drift = st.rand() - 0.5;
        s.0 += drift * 0.6;
        s.1 -= 0.55;
        s.2 -= 0.07;
    }
    sparks.retain(|s| s.2 > 0.0 && s.1 >= 0.0);
    sparks.truncate(80);
    for s in &sparks {
        m.set(s.0.round() as i32, s.1.round() as i32, s.2);
    }
    st.sparks = sparks;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scene_draws_something_with_music() {
        let bands = vec![0.6; 64];
        for scene in Scene::ALL {
            let mut m = Matrix::new(48, 20);
            draw(&mut m, scene, &bands, &mut SceneState::default());
            assert!(m.lit() > 10, "{scene:?} drew {} dots", m.lit());
        }
    }

    #[test]
    fn every_scene_reacts_to_music() {
        for scene in Scene::ALL {
            let mut quiet = Matrix::new(48, 20);
            let mut loud = Matrix::new(48, 20);
            draw(&mut quiet, scene, &[0.0; 64], &mut SceneState::default());
            draw(&mut loud, scene, &[0.9; 64], &mut SceneState::default());
            assert_ne!(quiet, loud, "{scene:?} ignores the audio");
        }
    }

    #[test]
    fn names_are_unique() {
        let mut names: Vec<_> = Scene::ALL.iter().map(|s| s.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), Scene::ALL.len());
    }

    #[test]
    fn horizon_keeps_a_short_history() {
        let mut st = SceneState::default();
        let mut m = Matrix::new(48, 20);
        for _ in 0..20 {
            draw(&mut m, Scene::Horizon, &[0.5; 64], &mut st);
        }
        assert_eq!(st.history.len(), 7);
    }

    #[test]
    fn phosphor_glow_fades_after_silence() {
        let mut st = SceneState::default();
        let mut m = Matrix::new(48, 20);
        draw(&mut m, Scene::Phosphor, &[1.0; 64], &mut st);
        let bright = m.dots.iter().sum::<f32>();
        for _ in 0..30 {
            draw(&mut m, Scene::Phosphor, &[0.0; 64], &mut st);
        }
        // Only the idle trace remains; the loud trace has decayed.
        assert!(m.dots.iter().sum::<f32>() < bright);
    }

    #[test]
    fn ember_peaks_fall_with_gravity() {
        let mut st = SceneState::default();
        let mut m = Matrix::new(48, 20);
        draw(&mut m, Scene::Ember, &[1.0; 64], &mut st);
        let top = st.peaks[0].0;
        draw(&mut m, Scene::Ember, &[0.0; 64], &mut st);
        let first = top - st.peaks[0].0;
        draw(&mut m, Scene::Ember, &[0.0; 64], &mut st);
        let second = top - first - st.peaks[0].0;
        assert!(second > first, "falls {first} then {second}");
    }
}
