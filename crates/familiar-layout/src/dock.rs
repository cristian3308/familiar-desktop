//! macOS-style dock magnification.
//!
//! Scales are computed from the distance between the pointer and each
//! icon's *resting* centre (the layout with no magnification). Using the
//! resting layout avoids the feedback loop where magnified icons move away
//! from the cursor and the dock jitters.

use crate::easing::cosine_bell;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DockMetrics {
    /// Resting icon edge length in logical pixels.
    pub icon_size: f32,
    /// Peak scale of the icon under the pointer (1.0 disables magnification).
    pub magnification: f32,
    /// Gap between slots in logical pixels.
    pub spacing: f32,
    /// Influence radius measured in resting icon slots.
    pub range_slots: f32,
    /// Width of a separator slot; separators never scale.
    pub separator_width: f32,
}

impl Default for DockMetrics {
    fn default() -> Self {
        Self { icon_size: 48.0, magnification: 1.45, spacing: 6.0, range_slots: 3.0, separator_width: 13.0 }
    }
}

impl DockMetrics {
    pub fn max_icon_size(&self) -> f32 {
        self.icon_size * self.magnification.max(1.0)
    }

    /// Width of the strip when nothing is magnified.
    pub fn resting_width(&self, kinds: &[SlotKind]) -> f32 {
        let sum: f32 = kinds.iter().map(|k| self.resting_extent(*k)).sum();
        sum + self.spacing * kinds.len().saturating_sub(1) as f32
    }

    /// Upper bound on the strip width, used to size the dock window once so
    /// it never has to be resized while the user is hovering.
    pub fn max_width(&self, kinds: &[SlotKind]) -> f32 {
        // Only the icons inside the influence radius grow; at most
        // `2 * range_slots + 1` of them, each by less than (mag - 1) * size.
        let icons = kinds.iter().filter(|k| **k == SlotKind::Icon).count() as f32;
        let growing = icons.min(2.0 * self.range_slots.ceil() + 1.0);
        self.resting_width(kinds) + growing * self.icon_size * (self.magnification - 1.0).max(0.0)
    }

    fn resting_extent(&self, kind: SlotKind) -> f32 {
        match kind {
            SlotKind::Icon => self.icon_size,
            SlotKind::Separator => self.separator_width,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotKind {
    Icon,
    Separator,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DockSlot {
    /// Left edge in window coordinates.
    pub x: f32,
    /// Edge length (icons are square; for separators this is the width).
    pub size: f32,
    pub scale: f32,
}

/// Lays out `kinds` centred in a window of `window_width`.
///
/// `pointer_x` is in window coordinates; `strength` (0..=1) fades the
/// magnification in and out when the pointer enters or leaves.
pub fn layout_dock(
    kinds: &[SlotKind],
    metrics: &DockMetrics,
    window_width: f32,
    pointer_x: Option<f32>,
    strength: f32,
) -> (Vec<DockSlot>, f32) {
    let strength = strength.clamp(0.0, 1.0);
    let resting_total = metrics.resting_width(kinds);
    let resting_start = (window_width - resting_total) / 2.0;
    let range_px = (metrics.range_slots * (metrics.icon_size + metrics.spacing)).max(1.0);

    let mut cursor = resting_start;
    let mut slots = Vec::with_capacity(kinds.len());
    let mut total = 0.0;
    for (i, kind) in kinds.iter().enumerate() {
        let extent = metrics.resting_extent(*kind);
        let centre = cursor + extent / 2.0;
        cursor += extent + metrics.spacing;

        let scale = match (kind, pointer_x) {
            (SlotKind::Icon, Some(px)) => {
                let falloff = cosine_bell((px - centre) / range_px);
                1.0 + (metrics.magnification - 1.0) * falloff * strength
            }
            _ => 1.0,
        };
        let size = extent * scale;
        slots.push(DockSlot { x: 0.0, size, scale });
        total += size;
        if i + 1 < kinds.len() {
            total += metrics.spacing;
        }
    }

    let mut x = (window_width - total) / 2.0;
    for slot in &mut slots {
        slot.x = x;
        x += slot.size + metrics.spacing;
    }
    (slots, total)
}

/// Returns the index of the slot whose horizontal extent contains `x`.
pub fn hit_test(slots: &[DockSlot], x: f32) -> Option<usize> {
    slots.iter().position(|s| x >= s.x && x < s.x + s.size)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn icons(n: usize) -> Vec<SlotKind> {
        vec![SlotKind::Icon; n]
    }

    #[test]
    fn resting_layout_is_centred_and_unscaled() {
        let m = DockMetrics::default();
        let kinds = icons(5);
        let (slots, total) = layout_dock(&kinds, &m, 1000.0, None, 1.0);
        assert!(slots.iter().all(|s| s.scale == 1.0 && s.size == 48.0));
        assert!((total - m.resting_width(&kinds)).abs() < 1e-3);
        let left = slots[0].x;
        let right = 1000.0 - (slots[4].x + slots[4].size);
        assert!((left - right).abs() < 1e-3);
    }

    #[test]
    fn icon_under_pointer_is_largest_and_neighbours_symmetric() {
        let m = DockMetrics::default();
        let kinds = icons(7);
        let (rest, _) = layout_dock(&kinds, &m, 1000.0, None, 1.0);
        let centre = rest[3].x + rest[3].size / 2.0;
        let (slots, _) = layout_dock(&kinds, &m, 1000.0, Some(centre), 1.0);
        assert!((slots[3].scale - m.magnification).abs() < 1e-4);
        assert!((slots[2].scale - slots[4].scale).abs() < 1e-4);
        assert!(slots[2].scale > slots[1].scale);
        assert!(slots.iter().all(|s| s.scale <= m.magnification + 1e-4));
    }

    #[test]
    fn strength_zero_disables_magnification() {
        let m = DockMetrics::default();
        let (slots, _) = layout_dock(&icons(4), &m, 600.0, Some(300.0), 0.0);
        assert!(slots.iter().all(|s| s.scale == 1.0));
    }

    #[test]
    fn separators_never_scale() {
        let m = DockMetrics::default();
        let kinds = [SlotKind::Icon, SlotKind::Separator, SlotKind::Icon];
        let (rest, _) = layout_dock(&kinds, &m, 400.0, None, 1.0);
        let sep_centre = rest[1].x + rest[1].size / 2.0;
        let (slots, _) = layout_dock(&kinds, &m, 400.0, Some(sep_centre), 1.0);
        assert_eq!(slots[1].scale, 1.0);
        assert_eq!(slots[1].size, m.separator_width);
    }

    #[test]
    fn max_width_bounds_every_pointer_position() {
        let m = DockMetrics::default();
        let kinds = icons(12);
        let bound = m.max_width(&kinds);
        for px in (0..1400).step_by(7) {
            let (_, total) = layout_dock(&kinds, &m, 1400.0, Some(px as f32), 1.0);
            assert!(total <= bound + 1e-3, "{total} > {bound} at {px}");
        }
    }

    #[test]
    fn hit_test_finds_slot() {
        let m = DockMetrics::default();
        let (slots, _) = layout_dock(&icons(3), &m, 300.0, None, 1.0);
        assert_eq!(hit_test(&slots, slots[1].x + 1.0), Some(1));
        assert_eq!(hit_test(&slots, 0.0), None);
    }
}
