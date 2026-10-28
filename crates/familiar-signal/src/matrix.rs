use crate::font::{self, GLYPH_H, GLYPH_W};

/// A dot-matrix frame: `cols * rows` intensities in `0.0..=1.0`, row-major.
#[derive(Clone, Debug, PartialEq)]
pub struct Matrix {
    pub cols: usize,
    pub rows: usize,
    pub dots: Vec<f32>,
}

impl Matrix {
    pub fn new(cols: usize, rows: usize) -> Self {
        Self { cols, rows, dots: vec![0.0; cols * rows] }
    }

    pub fn clear(&mut self) {
        self.dots.fill(0.0);
    }

    pub fn get(&self, x: usize, y: usize) -> f32 {
        self.dots[y * self.cols + x]
    }

    pub fn set(&mut self, x: i32, y: i32, v: f32) {
        if x >= 0 && y >= 0 && (x as usize) < self.cols && (y as usize) < self.rows {
            let i = y as usize * self.cols + x as usize;
            self.dots[i] = self.dots[i].max(v.clamp(0.0, 1.0));
        }
    }

    pub fn lit(&self) -> usize {
        self.dots.iter().filter(|d| **d > 0.0).count()
    }

    /// Writes `text` with its top-left at `(x, y)`.
    pub fn draw_text(&mut self, text: &str, x: i32, y: i32, intensity: f32) {
        let mut cx = x;
        for c in text.chars() {
            let g = font::glyph(c);
            for (row, bits) in g.iter().enumerate() {
                for col in 0..GLYPH_W {
                    if bits & (0x10 >> col) != 0 {
                        self.set(cx + col as i32, y + row as i32, intensity);
                    }
                }
            }
            cx += GLYPH_W as i32 + 1;
        }
    }

    /// Writes `text` centred in the frame.
    pub fn draw_text_centered(&mut self, text: &str, intensity: f32) {
        let w = font::text_width(text) as i32;
        let x = (self.cols as i32 - w) / 2;
        let y = (self.rows as i32 - GLYPH_H as i32) / 2;
        self.draw_text(text, x, y, intensity);
    }

    /// Spectrum bars growing from the bottom. `bands` are resampled to the
    /// column count; the top dot of each bar is partially lit for smoothness.
    pub fn draw_bars(&mut self, bands: &[f32]) {
        if bands.is_empty() {
            return;
        }
        for col in 0..self.cols {
            let lo = col * bands.len() / self.cols;
            let hi = ((col + 1) * bands.len() / self.cols).max(lo + 1);
            let level = bands[lo..hi].iter().copied().fold(0.0f32, f32::max);
            let height = level.clamp(0.0, 1.0) * self.rows as f32;
            let full = height.floor() as usize;
            for k in 0..full.min(self.rows) {
                self.set(col as i32, (self.rows - 1 - k) as i32, 1.0);
            }
            if full < self.rows {
                let frac = height - full as f32;
                if frac > 0.05 {
                    self.set(col as i32, (self.rows - 1 - full) as i32, frac);
                }
            }
        }
    }

    /// The SIGNAL/01 face: two eyes that hop with the bass and a mouth that
    /// becomes a small equaliser. With silent `bands` it renders a calm face.
    pub fn draw_signal_face(&mut self, bands: &[f32]) {
        let avg = |r: std::ops::Range<usize>| -> f32 {
            let r = r.start.min(bands.len())..r.end.min(bands.len());
            if r.is_empty() { 0.0 } else { bands[r.clone()].iter().sum::<f32>() / r.len() as f32 }
        };
        let bass = avg(0..8);
        let cols = self.cols as i32;
        let rows = self.rows as i32;
        let eye_w = (cols / 8).max(2);
        let eye_h = (rows / 4).max(2);
        let hop = (bass * eye_h as f32 * 0.8).round() as i32;
        let eye_y = rows / 4 - hop;
        let left = cols / 2 - cols / 5 - eye_w / 2;
        let right = cols / 2 + cols / 5 - eye_w / 2;
        for x in 0..eye_w {
            for y in 0..eye_h {
                self.set(left + x, eye_y + y, 1.0);
                self.set(right + x, eye_y + y, 1.0);
            }
        }

        // Mouth: an equaliser centred under the eyes.
        let mouth_w = cols / 2;
        let mouth_x = (cols - mouth_w) / 2;
        let base_y = rows - rows / 5;
        let max_h = (rows / 3).max(1);
        let energy = avg(0..bands.len());
        for i in 0..mouth_w {
            let band = (i as usize * bands.len().max(1)) / mouth_w.max(1) as usize;
            let level = bands.get(band).copied().unwrap_or(0.0);
            let h = if energy < 0.02 {
                // Silent: a gentle smile.
                let t = (i as f32 / (mouth_w - 1).max(1) as f32) * 2.0 - 1.0;
                (-(1.0 - t * t) * 2.0).round() as i32
            } else {
                -((level * max_h as f32).round() as i32)
            };
            let (from, to) = if h <= 0 { (base_y + h, base_y) } else { (base_y, base_y + h) };
            for y in from..=to {
                self.set(mouth_x + i, y, if y == from { 1.0 } else { 0.6 });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_renders_glyph_dots() {
        let mut m = Matrix::new(29, 7);
        m.draw_text("12:45", 0, 0, 1.0);
        // "1" has a dot at the top of its stem.
        assert_eq!(m.get(2, 0), 1.0);
        assert!(m.lit() > 30);
    }

    #[test]
    fn bars_follow_levels() {
        let mut m = Matrix::new(4, 10);
        m.draw_bars(&[0.0, 0.5, 1.0, 0.25]);
        let column = |c: usize| (0..10).filter(|r| m.get(c, *r) == 1.0).count();
        assert_eq!(column(0), 0);
        assert_eq!(column(1), 5);
        assert_eq!(column(2), 10);
        assert!(column(3) >= 2);
    }

    #[test]
    fn face_is_drawn_when_silent_and_moves_with_bass() {
        let mut calm = Matrix::new(32, 16);
        calm.draw_signal_face(&[0.0; 64]);
        assert!(calm.lit() > 10);
        let mut loud = Matrix::new(32, 16);
        loud.draw_signal_face(&[1.0; 64]);
        assert_ne!(calm, loud);
    }
}
