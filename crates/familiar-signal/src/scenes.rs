//! Music visualizer scenes drawn into a dot matrix.
//!
//! Every scene maps the 64 log-spaced band levels (0..=1) onto the frame;
//! `Peaks` keeps a little state (falling peak dots) in [`SceneState`].

use crate::Matrix;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scene {
    Bars,
    Mirror,
    Wave,
    Ring,
    Pulse,
    Peaks,
}

impl Scene {
    pub const ALL: [Scene; 6] = [Scene::Bars, Scene::Mirror, Scene::Wave, Scene::Ring, Scene::Pulse, Scene::Peaks];

    pub fn name(self) -> &'static str {
        match self {
            Scene::Bars => "BARRAS",
            Scene::Mirror => "ESPEJO",
            Scene::Wave => "ONDA",
            Scene::Ring => "ANILLO",
            Scene::Pulse => "PULSO",
            Scene::Peaks => "PICOS",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SceneState {
    peaks: Vec<f32>,
    /// Advances every frame; animates idle motion.
    tick: u32,
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
    bands.iter().take(n).sum::<f32>() / n as f32
}

pub fn draw(m: &mut Matrix, scene: Scene, bands: &[f32], st: &mut SceneState) {
    m.clear();
    st.tick = st.tick.wrapping_add(1);
    let (cols, rows) = (m.cols, m.rows);
    let (w, h) = (cols as i32, rows as i32);
    match scene {
        Scene::Bars => {
            for x in 0..cols {
                let v = level(bands, x, cols) * rows as f32;
                for y in 0..v.ceil() as i32 {
                    let fade = if (y as f32) + 1.0 > v { v.fract().max(0.3) } else { 1.0 };
                    m.set(x as i32, h - 1 - y, fade);
                }
            }
        }
        Scene::Mirror => {
            let mid = h / 2;
            for x in 0..cols {
                let v = level(bands, x, cols) * (rows as f32 / 2.0);
                for y in 0..=v as i32 {
                    let k = 1.0 - y as f32 / (rows as f32 / 2.0 + 1.0) * 0.5;
                    m.set(x as i32, mid - y, k);
                    m.set(x as i32, mid + y, k * 0.6);
                }
            }
        }
        Scene::Wave => {
            // A smooth line through the band levels, plus a soft trail below.
            let mut prev: Option<i32> = None;
            for x in 0..cols {
                let a = level(bands, x, cols);
                let b = level(bands, (x + 1).min(cols - 1), cols);
                let v = (a + b) / 2.0;
                let wobble = ((x as f32 * 0.35 + st.tick as f32 * 0.15).sin() * 0.08) + 0.5;
                let y = ((1.0 - (wobble * 0.4 + v * 0.6)) * (h - 1) as f32).round() as i32;
                m.set(x as i32, y, 1.0);
                if let Some(p) = prev {
                    for yy in p.min(y)..=p.max(y) {
                        m.set(x as i32, yy, 0.8);
                    }
                }
                m.set(x as i32, y + 1, 0.25);
                prev = Some(y);
            }
        }
        Scene::Ring => {
            let (cx, cy) = (w as f32 / 2.0 - 0.5, h as f32 / 2.0 - 0.5);
            let r0 = rows as f32 * 0.22;
            let spokes = 48;
            for s in 0..spokes {
                let ang = s as f32 / spokes as f32 * std::f32::consts::TAU;
                let v = level(bands, s, spokes);
                let len = r0 + v * rows as f32 * 0.3;
                let mut r = r0;
                while r <= len {
                    // Dots are square on screen, so x is not stretched.
                    m.set((cx + ang.cos() * r).round() as i32, (cy + ang.sin() * r).round() as i32, 1.0);
                    r += 0.7;
                }
            }
        }
        Scene::Pulse => {
            let (cx, cy) = (w as f32 / 2.0 - 0.5, h as f32 / 2.0 - 0.5);
            let b = bass(bands);
            let rings = [0.18 + b * 0.35, 0.35 + b * 0.4, 0.55 + b * 0.35];
            for (k, f) in rings.iter().enumerate() {
                let r = f * rows as f32;
                let steps = (r * 7.0).max(12.0) as i32;
                for i in 0..steps {
                    let ang = i as f32 / steps as f32 * std::f32::consts::TAU;
                    m.set(
                        (cx + ang.cos() * r).round() as i32,
                        (cy + ang.sin() * r).round() as i32,
                        1.0 - k as f32 * 0.3,
                    );
                }
            }
            m.set(cx.round() as i32, cy.round() as i32, 1.0);
        }
        Scene::Peaks => {
            st.peaks.resize(cols, 0.0);
            for x in 0..cols {
                let v = level(bands, x, cols) * rows as f32;
                st.peaks[x] = if v >= st.peaks[x] { v } else { (st.peaks[x] - 0.35).max(0.0) };
                for y in 0..v as i32 {
                    m.set(x as i32, h - 1 - y, 0.55);
                }
                if st.peaks[x] > 0.5 {
                    m.set(x as i32, h - 1 - st.peaks[x] as i32, 1.0);
                }
            }
        }
    }
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
}
