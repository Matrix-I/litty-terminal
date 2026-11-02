//! Per-pane scrollbar: thumb geometry (shared with mouse dragging) and its
//! dedicated instance, appended after the cells and drawn by the shader.

use super::super::Instance;

/// Dedicated scrollbar instance (preview + thumb bits); upper bits store height.
pub(in crate::render::renderer) const SCROLLBAR_OVERLAY: u32 = 0x30;
pub const SCROLLBAR_WIDTH: f64 = 10.0;

/// Thumb position within the track and height, shared by drawing and dragging.
pub fn scrollbar_thumb(track_height: f64, rows: usize, history: usize, offset: usize) -> (f64, f64) {
    let track_height = track_height.max(0.0);
    let total = (history + rows).max(1) as f64;
    let height = (track_height * rows as f64 / total).max(SCROLLBAR_WIDTH).min(track_height).round().min(track_height);
    let travel = (track_height - height).max(0.0);
    let y = travel * history.saturating_sub(offset) as f64 / history.max(1) as f64;
    (y, height)
}

/// The pane's scrollbar instance: x / y as f32 bits in `glyph` / `fg`, the
/// thumb height above the 0x30 tag in `bg` (0 = hidden at the bottom).
#[inline]
pub(in crate::render::renderer) fn instance(right: f64, y: f64, height: f64, offset: usize) -> Instance {
    Instance {
        glyph: ((right - SCROLLBAR_WIDTH) as f32).to_bits(),
        fg: (y as f32).to_bits(),
        bg: SCROLLBAR_OVERLAY | (if offset > 0 { (height as u32).min(0xFF_FFFF) } else { 0 } << 8),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_packs_position_and_hides_thumb_at_bottom() {
        let i = instance(100.0, 20.0, 40.0, 3);
        assert_eq!(f32::from_bits(i.glyph), 90.0, "x = right edge - width");
        assert_eq!(f32::from_bits(i.fg), 20.0);
        assert_eq!(i.bg, SCROLLBAR_OVERLAY | (40 << 8));
        assert_eq!(instance(100.0, 20.0, 40.0, 0).bg, SCROLLBAR_OVERLAY, "no thumb when not scrolled back");
    }

    #[test]
    fn scrollbar_thumb_reaches_track_ends_and_keeps_caps_visible() {
        assert_eq!(scrollbar_thumb(400.0, 40, 360, 360), (0.0, 40.0));
        assert_eq!(scrollbar_thumb(400.0, 40, 360, 0), (360.0, 40.0));
        assert_eq!(scrollbar_thumb(400.0, 40, 360, 180), (180.0, 40.0));
        assert_eq!(scrollbar_thumb(400.0, 40, 100_000, 100_000), (0.0, 10.0));
        assert_eq!(scrollbar_thumb(6.0, 40, 100_000, 1), (0.0, 6.0));
        assert_eq!(scrollbar_thumb(0.0, 0, 0, 0), (0.0, 0.0));
    }
}
