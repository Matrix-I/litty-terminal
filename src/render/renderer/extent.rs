//! Window size bookkeeping, kept apart from the drawable size.

/// A window's physical size and whether its drawables are shrunk. A hidden
/// window releases its drawables (1x1) but keeps its size, so the grid and its
/// shells keep their dimensions until the window is shown again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Extent {
    width: u32,
    height: u32,
    shrunk: bool,
}

impl Extent {
    pub(super) fn new(width: u32, height: u32) -> Self {
        Self { width: width.max(1), height: height.max(1), shrunk: false }
    }

    /// Take a new window size and restore full-size drawables. A zero size
    /// changes nothing and returns `false`.
    pub(super) fn resize(&mut self, width: u32, height: u32) -> bool {
        if width == 0 || height == 0 {
            return false;
        }
        *self = Self { width, height, shrunk: false };
        true
    }

    /// Release the drawables; the window size stays.
    pub(super) fn shrink(&mut self) {
        self.shrunk = true;
    }

    /// Physical window size in pixels.
    pub(super) fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Size the surface's drawables are configured with.
    pub(super) fn drawable_size(&self) -> (u32, u32) {
        if self.shrunk { (1, 1) } else { self.size() }
    }

    /// Columns and rows of `cell` pixels that fit inside `pad` on each side.
    pub(super) fn grid_size(&self, pad: (u32, u32), cell: (u32, u32)) -> (usize, usize) {
        let w = self.width.saturating_sub(2 * pad.0);
        let h = self.height.saturating_sub(2 * pad.1);
        ((w / cell.0.max(1)).max(1) as usize, (h / cell.1.max(1)).max(1) as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAD: (u32, u32) = (8, 4);
    const CELL: (u32, u32) = (16, 32);

    #[test]
    fn shrinking_keeps_the_grid_size() {
        // A hidden window that resizes its shells (config reload, a pane
        // exiting) must not squeeze them to one column and one row.
        let mut extent = Extent::new(1616, 968);
        let before = extent.grid_size(PAD, CELL);
        extent.shrink();
        assert_eq!(extent.grid_size(PAD, CELL), before);
        assert_eq!(before, (100, 30));
        assert_eq!(extent.size(), (1616, 968));
    }

    #[test]
    fn shrinking_releases_the_drawables() {
        let mut extent = Extent::new(1616, 968);
        assert_eq!(extent.drawable_size(), (1616, 968));
        extent.shrink();
        assert_eq!(extent.drawable_size(), (1, 1));
    }

    #[test]
    fn resizing_restores_full_size_drawables() {
        let mut extent = Extent::new(1616, 968);
        extent.shrink();
        assert!(extent.resize(800, 600));
        assert_eq!(extent.drawable_size(), (800, 600));
        assert_eq!(extent.size(), (800, 600));
    }

    #[test]
    fn a_zero_size_changes_nothing() {
        let mut extent = Extent::new(1616, 968);
        extent.shrink();
        assert!(!extent.resize(0, 600));
        assert!(!extent.resize(800, 0));
        assert_eq!(extent.size(), (1616, 968));
        assert_eq!(extent.drawable_size(), (1, 1));
    }

    #[test]
    fn a_window_smaller_than_one_cell_still_has_one() {
        let extent = Extent::new(10, 10);
        assert_eq!(extent.grid_size(PAD, CELL), (1, 1));
    }
}
