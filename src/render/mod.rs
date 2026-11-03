//! GPU rendering: wgpu pipelines and the single instanced draw call, the glyph
//! atlas and shaper, font discovery, post effects, inline images and frame
//! timing.
//!
//! Layer rule (enforced by the `layering_tests` module): uses only `render`
//! and `terminal`, plus the tolerated edges listed there.

pub(crate) mod atlas;
pub(crate) mod fontdb;
pub(crate) mod frame_metrics;
pub(crate) mod image_layer;
pub(crate) mod postfx;
pub(crate) mod renderer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneRect {
    pub id: u64,
    pub col: usize,
    pub row: usize,
    pub cols: usize,
    pub rows: usize,
    pub has_border_right: bool,
    pub has_border_bottom: bool,
}
