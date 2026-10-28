//! "Shake to find cursor" detection.
//!
//! A shake is several rapid horizontal direction reversals, each covering a
//! minimum distance, inside a short time window.

#[derive(Debug)]
pub struct ShakeDetector {
    min_travel: f32,
    window_ms: u32,
    reversals_needed: usize,
    last_x: Option<f32>,
    direction: i8,
    travel: f32,
    reversals: Vec<u32>,
}

impl Default for ShakeDetector {
    fn default() -> Self {
        Self::new(60.0, 600, 4)
    }
}

impl ShakeDetector {
    pub fn new(min_travel: f32, window_ms: u32, reversals_needed: usize) -> Self {
        Self {
            min_travel,
            window_ms,
            reversals_needed,
            last_x: None,
            direction: 0,
            travel: 0.0,
            reversals: Vec::with_capacity(reversals_needed + 1),
        }
    }

    /// Feeds a pointer sample; returns `true` once when a shake completes.
    pub fn push(&mut self, x: f32, time_ms: u32) -> bool {
        let Some(last) = self.last_x.replace(x) else {
            return false;
        };
        let dx = x - last;
        if dx == 0.0 {
            return false;
        }
        let dir: i8 = if dx > 0.0 { 1 } else { -1 };
        if dir == self.direction {
            self.travel += dx.abs();
            return false;
        }
        // Direction changed: count it only if the previous leg was long enough.
        if self.direction != 0 && self.travel >= self.min_travel {
            self.reversals.push(time_ms);
        }
        self.direction = dir;
        self.travel = dx.abs();
        self.reversals.retain(|t| time_ms.wrapping_sub(*t) <= self.window_ms);
        if self.reversals.len() >= self.reversals_needed {
            self.reversals.clear();
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wiggle(d: &mut ShakeDetector, amplitude: f32, step_ms: u32, legs: usize) -> bool {
        let mut t = 0;
        let mut fired = false;
        let mut x = 0.0;
        for leg in 0..legs {
            let target = if leg % 2 == 0 { amplitude } else { 0.0 };
            for _ in 0..5 {
                x += (target - x) / 2.0;
                t += step_ms;
                fired |= d.push(x, t);
            }
        }
        fired
    }

    #[test]
    fn fast_wide_wiggle_is_a_shake() {
        let mut d = ShakeDetector::default();
        assert!(wiggle(&mut d, 200.0, 10, 8));
    }

    #[test]
    fn slow_or_small_motion_is_not() {
        let mut d = ShakeDetector::default();
        assert!(!wiggle(&mut d, 20.0, 10, 8));
        let mut d = ShakeDetector::default();
        assert!(!wiggle(&mut d, 200.0, 200, 8));
    }
}
