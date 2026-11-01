//! Overlay widgets drawn as cell instances on top of the panes. Each builder
//! borrows only the atlas, the queue and the instance slice (`Cells`), never
//! the whole `Renderer`.

use super::{Instance, Renderer};
use crate::atlas::Atlas;

pub(super) mod history_popup;
pub(super) mod inspector;
pub(super) mod search_box;
pub(super) mod tab_bar;

/// The window's cell instances and the glyph atlas they index.
pub(in crate::renderer) struct Cells<'a> {
    pub(in crate::renderer) atlas: &'a mut Atlas,
    pub(in crate::renderer) queue: &'a wgpu::Queue,
    pub(in crate::renderer) instances: &'a mut [Instance],
    /// Window width in cells: the row stride of `instances`.
    pub(in crate::renderer) cols: usize,
}

/// Window rows whose instances changed this frame; `lo..=hi` is uploaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::renderer) struct DirtyRows {
    pub(in crate::renderer) lo: usize,
    pub(in crate::renderer) hi: usize,
}

impl DirtyRows {
    /// Nothing dirty (`lo > hi`).
    pub(in crate::renderer) const NONE: Self = Self { lo: usize::MAX, hi: 0 };

    /// Widen the range to include rows `first..=last`.
    #[inline]
    pub(in crate::renderer) fn mark(&mut self, first: usize, last: usize) {
        self.lo = self.lo.min(first);
        self.hi = self.hi.max(last);
    }
}

impl Renderer {
    /// Disjoint borrows of the fields every overlay writes through.
    pub(in crate::renderer) fn cells(&mut self, cols: usize) -> Cells<'_> {
        Cells { atlas: &mut self.atlas, queue: &self.queue, instances: &mut self.instances, cols }
    }
}

#[cfg(test)]
mod tests {
    use super::DirtyRows;

    #[test]
    fn dirty_rows_mark_widens_range() {
        let mut d = DirtyRows::NONE;
        assert!(d.lo > d.hi, "starts empty");
        d.mark(5, 7);
        d.mark(2, 3);
        assert_eq!(d, DirtyRows { lo: 2, hi: 7 });
    }
}
