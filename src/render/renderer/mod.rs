//! Direct Metal / Vulkan setup and the single-draw-call instanced renderer.
//!
//! Each frame: lock the grid briefly, rebuild instances only for dirty rows,
//! upload that contiguous range, then issue `draw(0..4, 0..cols*rows)`.

use std::sync::Arc;

use parking_lot::Mutex;
use crate::platform::gui::Window;

mod extent;
mod frame;
mod gpu;
mod pane_row;
use pane_row::RowScratch;
mod ui;
use gpu::{make_instance_buf, make_viewport_buffer};
pub use ui::scrollbar::{scrollbar_thumb, SCROLLBAR_WIDTH};
pub use ui::tab_bar::{slot_at as tab_slot_at, span as tab_span, TabBarView, WindowControl, MAX_SHIFTED_TABS};

use crate::render::atlas::{Atlas, FontSet};
use crate::render::backend::{
    Device, HostBuffer, Queue, RenderPipelines, Surface, SurfaceConfig,
};
use crate::terminal::grid::{Grid, Hyperlink};
use crate::render::HistoryView;
use crate::render::PaneRect;

pub struct RenderPane {
    #[allow(dead_code)]
    pub id: u64,
    pub grid: std::sync::Arc<Mutex<Grid>>,
    pub rect: PaneRect,
    pub is_active: bool,
}

/// Instance `bg` bit: the cell belongs to an unfocused split.
const UNFOCUSED: u32 = 0x40;
/// Instance `fg` bit: the slot is a color glyph (emoji) in the color atlas.
const COLOR_GLYPH: u32 = 0x04;

/// Per-cell GPU instance: 12 bytes, mirrors `Cell` with codepoint → atlas slot.
///
/// `glyph`: bits 0..16 atlas slot, 16..19 pane divider flags, 19..31 underline
/// color as 4-bit RGB, bit 31 set when that color is used. `bg` low byte: bits 0..3
/// underline style, bit 3 strikethrough, bit 4 preview row, bit 7 explicit color.
/// Extra scrollbar instances use low byte 0x30 and pack pixel height above it;
/// their `glyph` and `fg` fields hold the x/y position as f32 bits.
#[repr(C)]
#[derive(Copy, Clone, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct Instance {
    glyph: u32,
    fg: u32,
    bg: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    screen: [f32; 2],
    cell: [f32; 2],
    atlas_size: [f32; 2],
    offset: [f32; 2],
    cols: u32,
    slots_per_row: u32,
    cursor: [u32; 2],
    cursor_visible: u32,
    cursor_style: u32,
    default_fg: u32,
    default_bg: u32,
    tab_info: u32,
    bg_opacity: f32,
    /// Underline top and strikethrough center, in px from the cell top.
    underline_y: f32,
    strike_y: f32,
    /// OSC 12 cursor color as `0xRRGGBB00 | 1`, or 0 for the built-in color.
    cursor_color: u32,
    /// How much unfocused splits fade toward `unfocused_fill` (`1 - unfocused-split-opacity`).
    unfocused_dim: f32,
    /// `unfocused-split-fill` as `0xRRGGBB00 | 1`, or 0 for the default background.
    unfocused_fill: u32,
    /// `minimum-contrast` ratio between text and its background (1 = off).
    min_contrast: f32,
    /// Active pane's OSC 9;4 progress: `state << 8 | percent`, 0 = none.
    progress: u32,
    /// Active pane's first instance row, first column and width, for the progress bar.
    progress_row: u32,
    progress_col: u32,
    progress_cols: u32,
    /// `bell-features = border`: outline the window.
    bell_border: u32,
    /// 1 while drawing into a custom-shader texture (stored bottom row first).
    flip_y: u32,
    /// 0 = draw everything, 1 = backgrounds only, 2 = text only (images under the text).
    layer: u32,
    pane_count: u32,
    /// Tab being dragged plus one, or 0. Its pill is drawn above the others.
    tab_drag: u32,
    /// Pads `tab_shifts` to the 16-byte alignment WGSL requires of a uniform array.
    tab_pad: [u32; 3],
    /// Column shift of each of the first `MAX_SHIFTED_TABS` tabs, four per vector.
    tab_shifts: [[i32; 4]; MAX_SHIFTED_TABS / 4],
}

const _: () = assert!(std::mem::size_of::<Uniforms>() == 208);
const _: () = assert!(std::mem::size_of::<Instance>() == 12);

/// Tab bar height in cell heights; `shader.wgsl` uses the same 1.56.
const TAB_BAR_CELLS: f64 = 1.56;


/// One descriptor per pane, shared by its cells and its additional bottom row.
#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct Viewport {
    col: u32,
    row: u32,
    cols: u32,
    rows: u32,
    extra_start: u32,
    shift: f32,
    top: f32,
    bottom: f32,
    divider: u32,
    _pad: u32,
}

pub struct Renderer {
    frame_capture: Option<crate::render::frame_metrics::Capture>,
    /// Frames handed to the GPU so far; benchmarks compare it across a draw.
    frames_submitted: u64,
    /// Told right before each present (Wayland frame pacing). Weak: the app
    /// drops the renderer before its window, and never through this.
    window: std::sync::Weak<Window>,
    surface: Surface,
    device: Arc<Device>,
    queue: Arc<Queue>,
    config: SurfaceConfig,
    /// Window size; `config` holds the drawable size, 1x1 while shrunk.
    extent: extent::Extent,
    pipelines: RenderPipelines,
    instances: HostBuffer<Instance>,
    viewports: HostBuffer<Viewport>,
    atlas: Atlas,
    /// Window padding in physical pixels `(x, y)`, from `window-padding-x/y` points.
    pad: (u32, u32),
    pad_pt: (f32, f32),
    /// `window-padding-balance`: spread the leftover pixels evenly around the grid.
    pad_balance: bool,
    /// The window draws its own title bar band (client-side decorations).
    titlebar: bool,
    /// Rounded window corner radius in physical pixels (0 = square).
    corner_radius: f32,
    /// The surface composites with alpha, so cleared corners show what is behind the window.
    transparent_surface: bool,
    scale_factor: f64,
    bg_opacity: f32,
    cursor_style: u32,
    force_full: bool,
    last_cursor: Option<(usize, usize)>,
    last_cursor_visible: bool,
    /// Hovered link and the id of the pane it belongs to.
    last_hovered_link: Option<(u64, Hyperlink)>,
    /// IME composition text drawn at the cursor on the last frame.
    last_preedit: String,
    /// `cursor-color` (`0xRRGGBB00 | 1`, 0 = built-in), used when no OSC 12 is set.
    config_cursor_color: u32,
    /// `selection-background` / `selection-foreground` as instance colors.
    selection_colors: (u32, u32),
    /// Default fg / bg the instances were built against (the active pane's, which
    /// may come from OSC 10 / 11); a change rebuilds every row.
    last_defaults: (u32, u32),
    /// Pane drawn as focused on the last frame; a change rebuilds every row.
    last_active_pane: u64,
    /// Active pane's OSC 9;4 progress on the last frame.
    last_progress: Option<(u8, u8)>,
    /// A bell rang (`bell-features = border`) and has not been seen yet.
    bell_border: bool,
    /// Per-row scratch buffers reused across rows and frames.
    row_scratch: RowScratch,
    /// Inline images and this frame's placements (reused buffers).
    images: crate::render::image_layer::ImageLayer,
    image_draws: Vec<crate::terminal::graphics::ImageDraw>,
    placed: Vec<crate::render::image_layer::Placed>,
    /// `custom-shader` passes, if any.
    postfx: Option<crate::render::postfx::PostFx>,
    /// Shaders animate (`iTime`) and `custom-shader-animation` allows it.
    animate: bool,
    /// Mouse position in px, for the shaders' `iMouse`.
    mouse_px: (f32, f32),
    /// `unfocused-split-opacity`, `unfocused-split-fill`, `minimum-contrast`.
    unfocused_dim: f32,
    unfocused_fill: u32,
    min_contrast: f32,
}

impl Renderer {
    pub fn start_frame_capture(&mut self, occluded: bool) {
        let mut capture = crate::render::frame_metrics::Capture::new();
        capture.occluded(occluded);
        self.frame_capture = Some(capture);
    }

    pub fn capture_occlusion(&mut self, occluded: bool) {
        if let Some(capture) = &mut self.frame_capture { capture.occluded(occluded); }
    }

    pub fn finish_frame_capture(&mut self, label: &str, input_hz: u32) {
        if let Some(capture) = self.frame_capture.take() {
            println!("{}", capture.report(label, input_hz));
        }
    }

    /// Number of frames submitted to the GPU since the renderer was created.
    pub fn frames_submitted(&self) -> u64 {
        self.frames_submitted
    }

    /// Wait (bounded by `timeout`) until the GPU has finished the last submitted
    /// frame; `false` if it failed or timed out. Benchmarks only.
    pub fn wait_gpu_idle(&self, timeout: std::time::Duration) -> bool {
        self.surface.wait_gpu_idle(timeout)
    }

    /// Create the process-wide GPU device and queue before any window exists,
    /// so a startup thread can do it while the platform event loop starts. On
    /// macOS this compiles the built-in shaders (~300 ms on a cold shader cache).
    #[cfg(target_os = "macos")]
    pub fn prewarm_device() {
        let _ = gpu::shared_device();
    }

    pub fn new(
        window: Arc<Window>,
        font: Arc<FontSet>,
        font_px: f32,
        bg_opacity: f32,
        cursor_style: u32,
    ) -> Self {
        let size = window.inner_size();
        let (device, queue) = gpu::shared_device();
        let mut surface = Surface::new(&window, &device);
        let format = surface.default_format();
        let transparent = bg_opacity < 1.0;
        let extent = extent::Extent::new(size.width, size.height);
        let (width, height) = extent.drawable_size();
        let config = SurfaceConfig {
            width,
            height,
            format,
            transparent,
        };
        surface.configure(&device, &config);
        crate::platform::configure_layer_drawables(&window);

        let atlas = Atlas::new(&device, &queue, font, font_px);
        let pipelines = gpu::shared_pipeline(&device, format);
        let viewports = make_viewport_buffer(&device, 1);
        let instances = make_instance_buf(&device, 1);
        let scale_factor = window.scale_factor();
        let pad_px = (scale_factor * 4.0).round() as u32;

        let images = crate::render::image_layer::ImageLayer::new(&device, format);
        Renderer {
            window: Arc::downgrade(&window),
            frame_capture: None,
            frames_submitted: 0,
            surface,
            device,
            queue,
            config,
            extent,
            pipelines,
            instances,
            viewports,
            atlas,
            pad: (pad_px, pad_px),
            pad_pt: (4.0, 4.0),
            pad_balance: false,
            titlebar: false,
            corner_radius: 0.0,
            transparent_surface: transparent,
            scale_factor,
            bg_opacity: bg_opacity.clamp(0.0, 1.0),
            cursor_style,
            force_full: true,
            last_cursor: None,
            last_cursor_visible: true,
            last_hovered_link: None,
            last_preedit: String::new(),
            last_defaults: (0, 0),
            last_active_pane: 0,
            last_progress: None,
            bell_border: false,
            row_scratch: RowScratch::default(),
            images,
            image_draws: Vec::new(),
            placed: Vec::new(),
            postfx: None,
            animate: false,
            mouse_px: (0.0, 0.0),
            unfocused_dim: 0.3,
            unfocused_fill: 0,
            min_contrast: 1.0,
            config_cursor_color: 0,
            selection_colors: (0x3D59_A180, 0xFFFF_FF80),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabBarAction {
    Select(usize),
    CloseTab(usize),
    NewTab,
    /// A window button of the title bar.
    Control(WindowControl),
    /// Empty title bar: drags the window, double-click maximizes.
    TitleBar,
}

impl Renderer {
    /// Offset in pixels of a window position inside the cell under it (for SGR
    /// pixel mouse reports).
    pub fn offset_in_cell(&self, x_px: f64, y_px: f64, bands: usize) -> (usize, usize) {
        let (ox, oy) = self.origin();
        let top_y = (oy + self.atlas.cell_h * 2 * bands as u32) as f64;
        let rel_x = (x_px - ox as f64).max(0.0) as u32;
        let rel_y = (y_px - top_y).max(0.0) as u32;
        ((rel_x % self.atlas.cell_w.max(1)) as usize, (rel_y % self.atlas.cell_h.max(1)) as usize)
    }

    /// Height in pixels of the tab bar band at the top of the window.
    pub fn tab_bar_height(&self) -> f64 {
        f64::from(self.atlas.cell_h) * TAB_BAR_CELLS
    }

    /// Columns the tab strip shares between tabs (the rest holds the New Tab button).
    pub fn tab_cols_total(&self) -> usize {
        ui::tab_bar::strip_columns(self.grid_size().0)
    }

    /// Height in pixels of everything drawn above the terminal: title bar band and tab strip.
    pub fn chrome_height(&self) -> f64 {
        self.tab_strip_top() + self.tab_bar_height()
    }

    /// Top of the tab strip in pixels: below the title bar band when there is one.
    pub fn tab_strip_top(&self) -> f64 {
        if self.titlebar {
            f64::from(self.origin().1 + self.atlas.cell_h * 2)
        } else {
            0.0
        }
    }

    /// Draw a title bar band (window title and buttons) above the tab strip. The
    /// surface then needs alpha so the window's rounded corners can be transparent.
    pub fn set_titlebar(&mut self, on: bool) {
        self.titlebar = on;
        self.reconfigure_alpha();
        self.force_full = true;
    }

    /// Round the window's corners by `radius_px` physical pixels (0 for square corners,
    /// as when maximized). Ignored when the surface cannot be transparent.
    pub fn set_corner_radius(&mut self, radius_px: f32) {
        if (self.corner_radius - radius_px).abs() > 0.01 {
            self.corner_radius = radius_px;
            self.force_full = true;
        }
    }

    /// Pick the surface alpha mode: transparent when the background is translucent or
    /// the window has rounded corners, when the platform offers one.
    fn reconfigure_alpha(&mut self) {
        let transparent = self.bg_opacity < 1.0 || self.titlebar;
        self.transparent_surface = transparent;
        if self.config.transparent != transparent {
            self.config.transparent = transparent;
            self.surface.configure(&self.device, &self.config);
        }
    }

    /// Fractional grid column under window x position `x_px` (negative left of the grid).
    pub fn column_at(&self, x_px: f64) -> f64 {
        (x_px - self.origin().0 as f64) / self.atlas.cell_w.max(1) as f64
    }

    pub fn cell_size(&self) -> (u32, u32) {
        (self.atlas.cell_w, self.atlas.cell_h)
    }

    /// Columns and rows that fit in the window, also while its drawables are shrunk.
    pub fn grid_size(&self) -> (usize, usize) {
        self.extent.grid_size(self.pad, self.cell_size())
    }

    /// Top-left pixel of the cell grid: the padding, plus half of the leftover
    /// space on each axis with `window-padding-balance`.
    fn origin(&self) -> (u32, u32) {
        if !self.pad_balance {
            return self.pad;
        }
        let (cols, rows) = self.grid_size();
        let (width, height) = self.extent.size();
        let spare_x = width.saturating_sub(2 * self.pad.0 + cols as u32 * self.atlas.cell_w);
        let spare_y = height.saturating_sub(2 * self.pad.1 + rows as u32 * self.atlas.cell_h);
        (self.pad.0 + spare_x / 2, self.pad.1 + spare_y / 2)
    }

    /// `cursor-color`, `selection-background` and `selection-foreground` (`0xRRGGBB00`).
    pub fn set_colors(&mut self, cursor: Option<u32>, selection_bg: Option<u32>, selection_fg: Option<u32>) {
        self.config_cursor_color = cursor.map_or(0, |c| c | 1);
        // Instances mark explicit colors with the low-byte bits the shader checks.
        self.selection_colors = (selection_bg.map_or(0x3D59_A180, |c| c | 0x80), selection_fg.map_or(0xFFFF_FF80, |c| c | 0x80));
        self.force_full = true;
    }

    /// Load `custom-shader` files (checked, broken ones reported and skipped).
    /// `animation`: `None` never, `Some(false)` when a shader uses time,
    /// `Some(true)` always.
    pub fn set_custom_shaders(&mut self, paths: &[std::path::PathBuf], animation: Option<bool>) {
        use crate::render::postfx;
        let mut problems = Vec::new();
        let (sources, uses_time) = postfx::load(paths, &mut problems);
        self.postfx = postfx::PostFx::new(&self.device, self.config.format, &sources, &mut problems);
        postfx::report(&problems);
        self.animate = self.postfx.is_some() && animation.is_some_and(|always| always || uses_time);
        self.force_full = true;
    }

    /// A custom shader wants a new frame on its own (`iTime`).
    pub fn animating(&self) -> bool {
        self.animate
    }

    pub fn set_mouse(&mut self, x: f64, y: f64) {
        self.mouse_px = (x as f32, y as f32);
    }

    /// Show or hide the bell border; returns whether it changed.
    pub fn set_bell_border(&mut self, on: bool) -> bool {
        let changed = self.bell_border != on;
        self.bell_border = on;
        self.force_full |= changed;
        changed
    }

    /// `unfocused-split-opacity` (0.15..=1), `unfocused-split-fill` and `minimum-contrast` (1..=21).
    pub fn set_contrast(&mut self, split_opacity: f32, split_fill: Option<u32>, min_contrast: f32) {
        self.unfocused_dim = 1.0 - split_opacity.clamp(0.15, 1.0);
        self.unfocused_fill = split_fill.map_or(0, |c| c | 1);
        self.min_contrast = min_contrast.clamp(1.0, 21.0);
    }

    /// `window-padding-x` / `window-padding-y` in points and `window-padding-balance`.
    pub fn set_padding(&mut self, x_pt: f32, y_pt: f32, balance: bool) {
        self.pad_pt = (x_pt, y_pt);
        self.pad_balance = balance;
        self.update_pad();
    }

    fn update_pad(&mut self) {
        let px = |pt: f32| (pt as f64 * self.scale_factor).round().max(0.0) as u32;
        self.pad = (px(self.pad_pt.0), px(self.pad_pt.1));
        self.force_full = true;
    }

    /// Columns and terminal PTY rows when accounting for `bands` bands (title bar,
    /// tab strip) at the top, each two rows tall including the gap below it.
    pub fn grid_size_for_tabs(&self, bands: usize) -> (usize, usize) {
        let (cols, total_rows) = self.grid_size();
        let rows = if bands > 0 && total_rows > 2 * bands {
            total_rows - 2 * bands
        } else {
            total_rows
        };
        (cols, rows)
    }

    pub fn invalidate(&mut self) {
        self.force_full = true;
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if !self.extent.resize(width, height) {
            return;
        }
        (self.config.width, self.config.height) = self.extent.drawable_size();
        self.surface.configure(&self.device, &self.config);
        self.force_full = true;
    }

    /// Prune unplaced GPU image textures that have been idle past KEEP_DURATION.
    /// Returns the deadline when the next texture will expire, if any remain.
    pub fn prune_idle_images(&mut self, now: std::time::Instant) -> Option<std::time::Instant> {
        self.images.prune_idle(now)
    }

    /// Release swapchain drawables and offscreen textures when the window is hidden or closed.
    pub fn shrink_drawables(&mut self) {
        self.images.clear();
        self.placed.clear();
        self.image_draws.clear();
        if let Some(fx) = &mut self.postfx {
            fx.shrink();
        }
        // The window size stays in `extent`: a hidden window still resizes its
        // shells (config reload, a pane exiting). `resize` restores the drawables.
        self.extent.shrink();
        (self.config.width, self.config.height) = self.extent.drawable_size();
        self.surface.shrink_drawables(&self.device, &self.queue);
    }

    #[allow(dead_code)]
    pub fn pad(&self) -> (u32, u32) {
        self.pad
    }

    pub fn set_bg_opacity(&mut self, opacity: f32) {
        let op = opacity.clamp(0.0, 1.0);
        if (self.bg_opacity - op).abs() > 0.001 {
            self.bg_opacity = op;
            self.reconfigure_alpha();
            self.force_full = true;
        }
    }

    pub fn set_cursor_style(&mut self, style: u32) {
        if self.cursor_style != style {
            self.cursor_style = style;
            self.force_full = true;
        }
    }

    pub fn set_fonts(&mut self, fonts: Arc<FontSet>, px: f32) {
        self.atlas.set_fonts(&self.device, &self.queue, fonts, px);
        self.force_full = true;
    }

    pub fn set_font_px(&mut self, px: f32, scale_factor: f64) {
        self.atlas.set_px(&self.device, &self.queue, px);
        self.scale_factor = scale_factor;
        self.update_pad();
    }

    /// Position and size of the history search popup `(x0, y0, width, height, visible_rows)`.
    pub fn history_popup_geometry(&self, search: &HistoryView<'_>) -> Option<(usize, usize, usize, usize, usize)> {
        if !search.is_open {
            return None;
        }
        let (cols, total_rows) = self.grid_size();
        ui::history_popup::geometry(cols, total_rows)
    }

    pub fn hit_test_tab_bar(&self, x_px: f64, y_px: f64, titles: &[String]) -> Option<TabBarAction> {
        let n = titles.len();
        let has_strip = !self.titlebar || n > 1;
        let bar_h = self.tab_bar_height();
        let col = ((x_px - self.origin().0 as f64).max(0.0) as u32 / self.atlas.cell_w.max(1)) as usize;

        // Title bar band: window buttons, the rest drags the window.
        if self.titlebar && (0.0..bar_h).contains(&y_px) {
            let cols = self.grid_size().0;
            return Some(match ui::tab_bar::control_at(cols, col) {
                Some(control) => TabBarAction::Control(control),
                None => TabBarAction::TitleBar,
            });
        }
        let strip_top = self.tab_strip_top();
        if !has_strip || y_px < strip_top || y_px >= strip_top + bar_h {
            return None;
        }
        let tab_cols_total = self.tab_cols_total();
        if col >= tab_cols_total {
            return Some(TabBarAction::NewTab);
        }
        let tab_i = ui::tab_bar::slot_at(tab_cols_total, n, col);
        let (c0, c1) = ui::tab_bar::span(tab_cols_total, n, tab_i);
        let span = c1.saturating_sub(c0);

        // Click close button ('✕') at left corner of tab (c0..=c0+2)
        if ui::tab_bar::shows_close(span) && col <= c0 + 2 {
            return Some(TabBarAction::CloseTab(tab_i));
        }

        Some(TabBarAction::Select(tab_i))
    }

    /// Physical pixel position of the top-left corner of terminal cell `(row, col)`
    /// (the inverse of [`Renderer::pixel_to_cell`]).
    pub fn cell_origin_px(&self, row: usize, col: usize, bands: usize) -> (f64, f64) {
        let (ox, oy) = self.origin();
        let top_y = oy + self.atlas.cell_h * 2 * bands as u32;
        let x = ox as f64 + (col as u32 * self.atlas.cell_w) as f64;
        let y = top_y as f64 + (row as u32 * self.atlas.cell_h) as f64;
        (x, y)
    }

    /// Outer panes extend through window padding; inner panes end at their divider.
    pub fn scrollbar_track(&self, rect: &PaneRect, bands: usize) -> (f64, f64, f64) {
        let (cols, rows) = self.grid_size_for_tabs(bands);
        let (_, top) = self.cell_origin_px(rect.row, rect.col, bands);
        let (right, bottom) = self.cell_origin_px(rect.row + rect.rows, rect.col + rect.cols, bands);
        let (width, height) = self.extent.size();
        let right = if rect.col + rect.cols >= cols { width as f64 } else { right };
        let bottom = if rect.row + rect.rows >= rows { height as f64 } else { bottom };
        (right, top, bottom)
    }

    /// Map a physical window cursor position `(x_px, y_px)` to a terminal grid
    /// cell `(row, col)`. Returns `None` if the cursor is inside the tab bar.
    pub fn pixel_to_cell(
        &self,
        x_px: f64,
        y_px: f64,
        bands: usize,
        cols: usize,
        rows: usize,
    ) -> Option<(usize, usize)> {
        if cols == 0 || rows == 0 {
            return None;
        }
        let top_y = (self.origin().1 + self.atlas.cell_h * 2 * bands as u32) as f64;
        // The last band's content (not the gap below it) is chrome.
        if bands > 0 && y_px < top_y - f64::from(self.atlas.cell_h) * 2.0 + self.tab_bar_height() {
            return None;
        }
        let rel_x = (x_px - self.origin().0 as f64).max(0.0);
        let rel_y = (y_px - top_y).max(0.0);
        let col = ((rel_x as u32) / self.atlas.cell_w.max(1)) as usize;
        let row = ((rel_y as u32) / self.atlas.cell_h.max(1)) as usize;
        Some((row.min(rows - 1), col.min(cols - 1)))
    }




}


#[cfg(test)]
mod tests;
