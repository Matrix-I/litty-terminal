//! wgpu setup and the single-draw-call instanced renderer.
//!
//! Each frame: lock the grid briefly, rebuild instances only for dirty rows,
//! upload that contiguous range, then issue `draw(0..4, 0..cols*rows)`.

use std::sync::Arc;

use parking_lot::Mutex;
use winit::window::Window;

mod frame;
mod gpu;
mod pane_row;
use pane_row::RowScratch;
mod ui;
use gpu::{make_bind_group, make_instance_buf, make_viewport_buffer};
pub use ui::scrollbar::{scrollbar_thumb, SCROLLBAR_WIDTH};

use crate::atlas::{Atlas, FontSet};
use crate::terminal::grid::{Grid, Hyperlink};
use crate::terminal::history::HistorySearch;
use crate::split::PaneRect;

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
}

const _: () = assert!(std::mem::size_of::<Uniforms>() == 128);
const _: () = assert!(std::mem::size_of::<Instance>() == 12);


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
    frame_capture: Option<crate::frame_metrics::Capture>,
    surface: wgpu::Surface<'static>,
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    config: wgpu::SurfaceConfiguration,
    caps_alpha_modes: Vec<wgpu::CompositeAlphaMode>,
    pipeline: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    uniform_buf: wgpu::Buffer,
    /// Same as `bind_group` / `uniform_buf` for the text layer drawn over images with `z < 0`.
    text_bind_group: wgpu::BindGroup,
    text_uniform_buf: wgpu::Buffer,
    sampler: wgpu::Sampler,
    instance_buf: wgpu::Buffer,
    instances: Vec<Instance>,
    viewport_buf: wgpu::Buffer,
    viewports: Vec<Viewport>,
    atlas: Atlas,
    /// Window padding in physical pixels `(x, y)`, from `window-padding-x/y` points.
    pad: (u32, u32),
    pad_pt: (f32, f32),
    /// `window-padding-balance`: spread the leftover pixels evenly around the grid.
    pad_balance: bool,
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
    images: crate::image_layer::ImageLayer,
    image_draws: Vec<crate::terminal::graphics::ImageDraw>,
    placed: Vec<crate::image_layer::Placed>,
    /// `custom-shader` passes, if any.
    postfx: Option<crate::postfx::PostFx>,
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
        let mut capture = crate::frame_metrics::Capture::new();
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

    pub fn new(
        window: Arc<Window>,
        font: Arc<FontSet>,
        font_px: f32,
        bg_opacity: f32,
        cursor_style: u32,
    ) -> Self {
        let size = window.inner_size();

        let instance = gpu::global_instance();
        let surface = instance.create_surface(window.clone()).expect("create surface");
        let (adapter, device, queue) = gpu::shared_device(&surface);

        // Prefer a non-sRGB format so packed colors pass straight through.
        let caps = surface.get_capabilities(&adapter);
        let format = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);
        let caps_alpha_modes = caps.alpha_modes.clone();
        let alpha_mode = if bg_opacity < 1.0 {
            caps_alpha_modes
                .iter()
                .copied()
                .find(|&m| m == wgpu::CompositeAlphaMode::PreMultiplied || m == wgpu::CompositeAlphaMode::PostMultiplied)
                .unwrap_or(caps_alpha_modes[0])
        } else {
            caps_alpha_modes[0]
        };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 1,
            alpha_mode,
            view_formats: vec![],
        };
        surface.configure(&device, &config);
        #[cfg(target_os = "macos")]
        crate::macos::configure_layer_drawables(&window);

        let atlas = Atlas::new(&device, &queue, font, font_px);

        let (bgl, pipeline) = gpu::shared_pipeline(&device, format);

        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let viewport_buf = make_viewport_buffer(&device, 1);
        let bind_group = make_bind_group(&device, &bgl, &uniform_buf, &atlas, &sampler, &viewport_buf);
        let text_uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("text uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let text_bind_group = make_bind_group(&device, &bgl, &text_uniform_buf, &atlas, &sampler, &viewport_buf);
        let instance_buf = make_instance_buf(&device, 1);
        let scale_factor = window.scale_factor();
        let pad_px = (scale_factor * 4.0).round() as u32;

        let images = crate::image_layer::ImageLayer::new(&device, format);
        Renderer {
            frame_capture: None,
            surface,
            device,
            queue,
            config,
            caps_alpha_modes,
            pipeline,
            bgl,
            bind_group,
            uniform_buf,
            text_bind_group,
            text_uniform_buf,
            sampler,
            instance_buf,
            instances: Vec::new(),
            viewport_buf,
            viewports: Vec::new(),
            atlas,
            pad: (pad_px, pad_px),
            pad_pt: (4.0, 4.0),
            pad_balance: false,
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
}

impl Renderer {
    /// Offset in pixels of a window position inside the cell under it (for SGR
    /// pixel mouse reports).
    pub fn offset_in_cell(&self, x_px: f64, y_px: f64, show_tab_bar: bool) -> (usize, usize) {
        let (ox, oy) = self.origin();
        let top_y = if show_tab_bar { (oy + self.atlas.cell_h * 2) as f64 } else { oy as f64 };
        let rel_x = (x_px - ox as f64).max(0.0) as u32;
        let rel_y = (y_px - top_y).max(0.0) as u32;
        ((rel_x % self.atlas.cell_w.max(1)) as usize, (rel_y % self.atlas.cell_h.max(1)) as usize)
    }

    pub fn cell_size(&self) -> (u32, u32) {
        (self.atlas.cell_w, self.atlas.cell_h)
    }

    /// Columns and rows that fit in the current surface.
    pub fn grid_size(&self) -> (usize, usize) {
        let w = self.config.width.saturating_sub(2 * self.pad.0);
        let h = self.config.height.saturating_sub(2 * self.pad.1);
        ((w / self.atlas.cell_w).max(1) as usize, (h / self.atlas.cell_h).max(1) as usize)
    }

    /// Top-left pixel of the cell grid: the padding, plus half of the leftover
    /// space on each axis with `window-padding-balance`.
    fn origin(&self) -> (u32, u32) {
        if !self.pad_balance {
            return self.pad;
        }
        let (cols, rows) = self.grid_size();
        let spare_x = self.config.width.saturating_sub(2 * self.pad.0 + cols as u32 * self.atlas.cell_w);
        let spare_y = self.config.height.saturating_sub(2 * self.pad.1 + rows as u32 * self.atlas.cell_h);
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
        let (sources, uses_time) = crate::postfx::load(paths);
        self.postfx = crate::postfx::PostFx::new(&self.device, self.config.format, &sources);
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

    /// Columns and terminal PTY rows when accounting for the top tab bar + gap
    /// shown whenever 2 or more tabs are open.
    pub fn grid_size_for_tabs(&self, show_tab_bar: bool) -> (usize, usize) {
        let (cols, total_rows) = self.grid_size();
        let rows = if show_tab_bar && total_rows > 2 {
            total_rows - 2
        } else {
            total_rows
        };
        (cols, rows)
    }

    pub fn invalidate(&mut self) {
        self.force_full = true;
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.force_full = true;
    }

    /// Prune unplaced GPU image textures that have been idle past KEEP_DURATION.
    /// Returns the deadline when the next texture will expire, if any remain.
    pub fn prune_idle_images(&mut self, now: std::time::Instant) -> Option<std::time::Instant> {
        self.images.prune_idle(now)
    }

    /// Present a few 1×1 frames before the window is hidden or closed. macOS keeps the
    /// last drawables of a layer alive (about 30 MB for a Retina window),
    /// so leave it holding tiny ones instead.
    pub fn shrink_drawables(&mut self) {
        self.images.clear();
        self.placed.clear();
        self.image_draws.clear();
        if let Some(fx) = &mut self.postfx {
            fx.shrink();
        }
        self.config.width = 1;
        self.config.height = 1;
        self.surface.configure(&self.device, &self.config);
        for _ in 0..3 {
            let Ok(frame) = self.surface.get_current_texture() else { break };
            let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
            let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shrink_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            self.queue.submit([encoder.finish()]);
            frame.present();
        }
        let _ = self.device.poll(wgpu::Maintain::Wait);
    }

    #[allow(dead_code)]
    pub fn pad(&self) -> (u32, u32) {
        self.pad
    }

    pub fn set_bg_opacity(&mut self, opacity: f32) {
        let op = opacity.clamp(0.0, 1.0);
        if (self.bg_opacity - op).abs() > 0.001 {
            self.bg_opacity = op;
            let alpha_mode = if self.bg_opacity < 1.0 {
                self.caps_alpha_modes
                    .iter()
                    .copied()
                    .find(|&m| m == wgpu::CompositeAlphaMode::PreMultiplied || m == wgpu::CompositeAlphaMode::PostMultiplied)
                    .unwrap_or(self.caps_alpha_modes[0])
            } else {
                self.caps_alpha_modes[0]
            };
            self.config.alpha_mode = alpha_mode;
            self.surface.configure(&self.device, &self.config);
            self.force_full = true;
        }
    }

    pub fn set_cursor_style(&mut self, style: u32) {
        if self.cursor_style != style {
            self.cursor_style = style;
            self.force_full = true;
        }
    }

    fn rebind_atlas(&mut self) {
        self.bind_group = make_bind_group(&self.device, &self.bgl, &self.uniform_buf, &self.atlas, &self.sampler, &self.viewport_buf);
        self.text_bind_group =
            make_bind_group(&self.device, &self.bgl, &self.text_uniform_buf, &self.atlas, &self.sampler, &self.viewport_buf);
    }

    pub fn set_fonts(&mut self, fonts: Arc<FontSet>, px: f32) {
        self.atlas.set_fonts(&self.device, &self.queue, fonts, px);
        self.rebind_atlas();
        self.force_full = true;
    }

    pub fn set_font_px(&mut self, px: f32, scale_factor: f64) {
        self.atlas.set_px(&self.device, &self.queue, px);
        self.rebind_atlas();
        self.scale_factor = scale_factor;
        self.update_pad();
    }

    /// Position and size of the history search popup `(x0, y0, width, height, visible_rows)`.
    pub fn history_popup_geometry(&self, search: &HistorySearch) -> Option<(usize, usize, usize, usize, usize)> {
        if !search.is_open {
            return None;
        }
        let (cols, total_rows) = self.grid_size();
        ui::history_popup::geometry(cols, total_rows)
    }

    pub fn hit_test_tab_bar(&self, x_px: f64, y_px: f64, titles: &[String]) -> Option<TabBarAction> {
        if titles.len() <= 1 {
            return None;
        }
        let bar_h = (self.atlas.cell_h as f64) * 1.56;
        if y_px < 0.0 || y_px >= bar_h {
            return None;
        }
        let (cols, _) = self.grid_size();
        let tab_cols_total = cols.saturating_sub(2).max(1);
        let col = ((x_px - self.origin().0 as f64).max(0.0) as u32 / self.atlas.cell_w.max(1)) as usize;
        if col >= tab_cols_total {
            return Some(TabBarAction::NewTab);
        }
        let n = titles.len();
        let mut tab_i = (col * n) / tab_cols_total;
        let next_start = ((tab_i + 1) * tab_cols_total) / n;
        if col >= next_start && tab_i + 1 < n {
            tab_i += 1;
        }
        let tab_i = tab_i.min(n - 1);
        let c0 = (tab_i * tab_cols_total) / n;
        let c1 = ((tab_i + 1) * tab_cols_total) / n;
        let span = c1.saturating_sub(c0);

        // Click close button ('✕') at left corner of tab (c0..=c0+2)
        if span >= 6 && col <= c0 + 2 {
            return Some(TabBarAction::CloseTab(tab_i));
        }

        Some(TabBarAction::Select(tab_i))
    }

    /// Physical pixel position of the top-left corner of terminal cell `(row, col)`
    /// (the inverse of [`Renderer::pixel_to_cell`]).
    pub fn cell_origin_px(&self, row: usize, col: usize, show_tab_bar: bool) -> (f64, f64) {
        let (ox, oy) = self.origin();
        let top_y = if show_tab_bar { oy + self.atlas.cell_h * 2 } else { oy };
        let x = ox as f64 + (col as u32 * self.atlas.cell_w) as f64;
        let y = top_y as f64 + (row as u32 * self.atlas.cell_h) as f64;
        (x, y)
    }

    /// Outer panes extend through window padding; inner panes end at their divider.
    pub fn scrollbar_track(&self, rect: &PaneRect, show_tabs: bool) -> (f64, f64, f64) {
        let (cols, rows) = self.grid_size_for_tabs(show_tabs);
        let (_, top) = self.cell_origin_px(rect.row, rect.col, show_tabs);
        let (right, bottom) = self.cell_origin_px(rect.row + rect.rows, rect.col + rect.cols, show_tabs);
        let right = if rect.col + rect.cols >= cols { self.config.width as f64 } else { right };
        let bottom = if rect.row + rect.rows >= rows { self.config.height as f64 } else { bottom };
        (right, top, bottom)
    }

    /// Map a physical window cursor position `(x_px, y_px)` to a terminal grid
    /// cell `(row, col)`. Returns `None` if the cursor is inside the tab bar.
    pub fn pixel_to_cell(
        &self,
        x_px: f64,
        y_px: f64,
        show_tab_bar: bool,
        cols: usize,
        rows: usize,
    ) -> Option<(usize, usize)> {
        if cols == 0 || rows == 0 {
            return None;
        }
        if show_tab_bar && y_px < (self.atlas.cell_h as f64) * 1.56 {
            return None;
        }
        let top_y = if show_tab_bar {
            (self.origin().1 + self.atlas.cell_h * 2) as f64
        } else {
            self.origin().1 as f64
        };
        let rel_x = (x_px - self.origin().0 as f64).max(0.0);
        let rel_y = (y_px - top_y).max(0.0);
        let col = ((rel_x as u32) / self.atlas.cell_w.max(1)) as usize;
        let row = ((rel_y as u32) / self.atlas.cell_h.max(1)) as usize;
        Some((row.min(rows - 1), col.min(cols - 1)))
    }




}


#[cfg(test)]
mod tests;
