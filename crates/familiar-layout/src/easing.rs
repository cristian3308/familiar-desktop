//! Small easing helpers for animations driven from Rust timers.

/// Cosine bell: 1.0 at `t = 0`, 0.0 at `|t| >= 1`, smooth first derivative.
pub fn cosine_bell(t: f32) -> f32 {
    let t = t.abs();
    if t >= 1.0 { 0.0 } else { 0.5 * (1.0 + (std::f32::consts::PI * t).cos()) }
}

/// Gaussian falloff with `sigma` expressed in the same unit as `t`.
pub fn gaussian(t: f32, sigma: f32) -> f32 {
    (-(t * t) / (2.0 * sigma * sigma)).exp()
}

pub fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0) - 1.0;
    t * t * t + 1.0
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Moves `current` toward `target` by at most `max_step`.
pub fn approach(current: f32, target: f32, max_step: f32) -> f32 {
    if (target - current).abs() <= max_step {
        target
    } else if target > current {
        current + max_step
    } else {
        current - max_step
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bell_edges() {
        assert_eq!(cosine_bell(0.0), 1.0);
        assert_eq!(cosine_bell(1.0), 0.0);
        assert_eq!(cosine_bell(-2.0), 0.0);
        assert!((cosine_bell(0.5) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn approach_clamps() {
        assert_eq!(approach(0.0, 1.0, 0.25), 0.25);
        assert_eq!(approach(0.9, 1.0, 0.25), 1.0);
        assert_eq!(approach(1.0, 0.0, 0.5), 0.5);
    }
}
