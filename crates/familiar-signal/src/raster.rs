//! Software rasteriser for dot-matrix frames.

use crate::Matrix;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba(pub u8, pub u8, pub u8, pub u8);

#[derive(Clone, Copy, Debug)]
pub struct DotStyle {
    /// Distance between dot centres, in pixels.
    pub pitch: u32,
    /// Dot radius as a fraction of the pitch (0.5 = dots touch).
    pub radius: f32,
    pub on: Rgba,
    /// Colour of unlit dots; use alpha 0 to hide them.
    pub off: Rgba,
}

impl DotStyle {
    pub fn image_size(&self, m: &Matrix) -> (u32, u32) {
        (m.cols as u32 * self.pitch, m.rows as u32 * self.pitch)
    }
}

/// Renders `m` into a straight-alpha RGBA8 buffer (row-major, tightly packed)
/// and returns `(width, height, pixels)`.
pub fn render(m: &Matrix, style: &DotStyle) -> (u32, u32, Vec<u8>) {
    let (w, h) = style.image_size(m);
    let mut px = vec![0u8; (w * h * 4) as usize];
    let pitch = style.pitch as f32;
    let r = (style.radius * pitch).max(0.5);
    for row in 0..m.rows {
        for col in 0..m.cols {
            let v = m.get(col, row);
            let colour = mix(style.off, style.on, v);
            if colour.3 == 0 {
                continue;
            }
            let cx = col as f32 * pitch + pitch / 2.0;
            let cy = row as f32 * pitch + pitch / 2.0;
            let x0 = (cx - r - 1.0).floor().max(0.0) as u32;
            let y0 = (cy - r - 1.0).floor().max(0.0) as u32;
            let x1 = ((cx + r + 1.0).ceil() as u32).min(w);
            let y1 = ((cy + r + 1.0).ceil() as u32).min(h);
            for y in y0..y1 {
                for x in x0..x1 {
                    let dx = x as f32 + 0.5 - cx;
                    let dy = y as f32 + 0.5 - cy;
                    let coverage = (r + 0.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
                    if coverage > 0.0 {
                        let i = ((y * w + x) * 4) as usize;
                        px[i] = colour.0;
                        px[i + 1] = colour.1;
                        px[i + 2] = colour.2;
                        px[i + 3] = (colour.3 as f32 * coverage) as u8;
                    }
                }
            }
        }
    }
    (w, h, px)
}

fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Rgba(l(a.0, b.0), l(a.1, b.1), l(a.2, b.2), l(a.3, b.3))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lit_dot_is_opaque_at_centre_and_transparent_between() {
        let mut m = Matrix::new(2, 1);
        m.set(0, 0, 1.0);
        let style = DotStyle { pitch: 10, radius: 0.35, on: Rgba(255, 0, 0, 255), off: Rgba(0, 0, 0, 0) };
        let (w, h, px) = render(&m, &style);
        assert_eq!((w, h), (20, 10));
        let at = |x: u32, y: u32| px[((y * w + x) * 4 + 3) as usize];
        assert_eq!(at(5, 5), 255);
        assert_eq!(at(15, 5), 0);
        assert_eq!(at(0, 0), 0);
    }
}
