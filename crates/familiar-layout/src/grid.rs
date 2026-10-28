//! Snapping grid for desktop widgets.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WidgetGrid {
    /// Cell edge in logical pixels.
    pub cell: f32,
    /// Gap between cells.
    pub gap: f32,
    /// Margin from the screen edge.
    pub margin: f32,
    pub columns: i32,
    pub rows: i32,
}

impl WidgetGrid {
    /// Builds a grid that fills a work area of `width` x `height`.
    pub fn for_area(width: f32, height: f32, cell: f32, gap: f32, margin: f32) -> Self {
        let pitch = cell + gap;
        let columns = (((width - 2.0 * margin + gap) / pitch).floor() as i32).max(1);
        let rows = (((height - 2.0 * margin + gap) / pitch).floor() as i32).max(1);
        Self { cell, gap, margin, columns, rows }
    }

    fn pitch(&self) -> f32 {
        self.cell + self.gap
    }

    /// Top-left pixel of a grid cell.
    pub fn to_pixels(&self, gx: i32, gy: i32) -> (f32, f32) {
        (self.margin + gx as f32 * self.pitch(), self.margin + gy as f32 * self.pitch())
    }

    /// Pixel size of a widget spanning `w` x `h` units.
    pub fn span_pixels(&self, w: i32, h: i32) -> (f32, f32) {
        (
            w as f32 * self.cell + (w - 1).max(0) as f32 * self.gap,
            h as f32 * self.cell + (h - 1).max(0) as f32 * self.gap,
        )
    }

    /// Nearest cell for a widget whose top-left is at `(x, y)`, clamped so a
    /// `w` x `h` widget stays fully on screen.
    pub fn snap(&self, x: f32, y: f32, w: i32, h: i32) -> (i32, i32) {
        let gx = ((x - self.margin) / self.pitch()).round() as i32;
        let gy = ((y - self.margin) / self.pitch()).round() as i32;
        (gx.clamp(0, (self.columns - w).max(0)), gy.clamp(0, (self.rows - h).max(0)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_rounds_and_clamps() {
        let g = WidgetGrid::for_area(1920.0, 1040.0, 80.0, 16.0, 24.0);
        assert_eq!(g.snap(24.0, 24.0, 2, 2), (0, 0));
        assert_eq!(g.snap(24.0 + 96.0 * 2.4, 24.0, 2, 2), (2, 0));
        assert_eq!(g.snap(-500.0, 99999.0, 2, 2), (0, g.rows - 2));
    }

    #[test]
    fn round_trip() {
        let g = WidgetGrid::for_area(1280.0, 720.0, 64.0, 8.0, 16.0);
        let (x, y) = g.to_pixels(3, 2);
        assert_eq!(g.snap(x, y, 1, 1), (3, 2));
        assert_eq!(g.span_pixels(2, 1), (136.0, 64.0));
    }
}
