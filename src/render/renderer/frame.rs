//! One frame: decide whether anything changed, rebuild dirty instance rows,
//! draw overlays, upload, and submit the cell / image / custom-shader passes.

use super::gpu::{make_instance_buf, make_viewport_buffer};
use super::pane_row::{build_pane_row, PaneRowCtx};
use super::ui::{self, DirtyRows};
use super::{scrollbar_thumb, Instance, RenderPane, Renderer, TabBarView, Uniforms, Viewport, MAX_SHIFTED_TABS};
use crate::terminal::grid::Hyperlink;
use crate::render::HistoryView;
use crate::terminal::search::SearchBar;
use crate::render::PaneRect;

/// Instance-buffer layout of one frame.
#[derive(Clone, Copy)]
struct FrameLayout {
    cols: usize,
    /// 1 when the tab bar takes the first row.
    top_rows: usize,
    total_rows: usize,
    /// `cols * total_rows`: first instance after the window grid (extra bottom rows follow).
    grid_cells: usize,
    /// First scrollbar instance, after every pane's extra bottom row.
    scrollbar_start: usize,
    instance_count: usize,
    pane_count: usize,
}

/// What the active pane contributes to this frame, read under one short lock.
struct ActivePane {
    cursor: Option<(usize, usize)>,
    dirty: bool,
    defaults: (u32, u32),
    cursor_color: u32,
    progress: Option<(u8, u8)>,
    cursor_style: u32,
}

/// Per-frame inputs shared by the instance-building phases.
#[derive(Clone, Copy)]
struct FrameCtx<'a> {
    layout: FrameLayout,
    show_tabs: bool,
    tab_bar: Option<TabBarView<'a>>,
    hovered_link: Option<(u64, &'a Hyperlink)>,
    /// Active pane's default fg / bg (OSC 10 / 11), sent to the shader.
    defaults: (u32, u32),
    active_rect: Option<PaneRect>,
}

/// Overlay state passed through from the caller.
#[derive(Clone, Copy)]
struct OverlayInputs<'a> {
    preedit: &'a str,
    search: &'a SearchBar,
    history_search: Option<HistoryView<'a>>,
    inspector: Option<&'a [String]>,
}

/// Cell-based overlays stay fixed while open; drop any fractional scroll offset.
fn snap_fractional_scroll(panes: &[RenderPane]) {
        for pane in panes {
            let mut g = pane.grid.lock();
            if g.scroll_fraction > 0.0 { g.scroll_display(0); }
        }
}

impl Renderer {
    #[allow(clippy::too_many_arguments)]
    pub fn render_panes(
        &mut self,
        panes: &[RenderPane],
        tab_bar: Option<TabBarView<'_>>,
        hovered_link: Option<(u64, &Hyperlink)>,
        cursor_blink_on: bool,
        history_search: Option<HistoryView<'_>>,
        search: &SearchBar,
        preedit: &str,
        inspector: Option<&[String]>,
    ) {
        let capture_start = self.frame_capture.as_ref().map(|_| std::time::Instant::now());
        if panes.is_empty() {
            return;
        }
        self.note_overlay_inputs(inspector.is_some(), preedit, hovered_link);
        // Overlays are redrawn every frame on top of the cells.
        let is_search_open = history_search.is_some_and(|h| h.is_open) || search.is_open;
        // Cell-based overlays stay fixed while open; snap once when opening them.
        if is_search_open || inspector.is_some() {
            snap_fractional_scroll(panes);
        }
        let layout = self.frame_layout(panes, tab_bar.is_some());
        let active_pane = panes.iter().find(|p| p.is_active).unwrap_or(&panes[0]);
        let active = self.read_active_pane(active_pane, layout.top_rows);
        self.note_active_changes(&active, active_pane.id);
        let effective_visible = active.cursor.is_some() && cursor_blink_on;

        // Fast check before acquiring a Metal/Vulkan swapchain drawable
        if !self.force_full
            && !is_search_open
            && !self.animate
            && !tab_bar.is_some_and(|view| view.dirty)
            && self.nothing_changed(panes, active_pane.id, &active, &layout, effective_visible)
        {
            return;
        }

        let acquire_start = capture_start.map(|_| std::time::Instant::now());
        let Some(frame) = self.acquire_frame() else {
            // The tab bar's change flag was consumed by the caller; keep the rebuild pending.
            self.force_full |= tab_bar.is_some_and(|view| view.dirty);
            return;
        };
        let acquire_ms = acquire_start.map_or(0.0, |t| t.elapsed().as_secs_f64() * 1000.0);

        self.ensure_buffers(layout.instance_count, panes.len());
        self.viewports.clear();
        self.atlas.begin_frame();
        let full = self.force_full;
        let mut dirty = DirtyRows::NONE;
        let ctx = FrameCtx {
            layout,
            show_tabs: tab_bar.is_some(),
            tab_bar,
            hovered_link,
            defaults: active.defaults,
            active_rect: panes.iter().find(|p| p.is_active).map(|p| p.rect),
        };

        if let Some(view) = tab_bar
            && (full || view.dirty) {
                ui::tab_bar::build(&mut self.cells(layout.cols), view);
                dirty.mark(0, 0);
            }
        self.collect_pane_instances(panes, &ctx, full, &mut dirty);
        if self.atlas.take_evicted() {
            self.rebuild_after_eviction(panes, &ctx, &mut dirty);
        }

        self.force_full = false;
        self.last_cursor = active.cursor;

        let overlays = OverlayInputs { preedit, search, history_search, inspector };
        let cursor = self.collect_overlays(&ctx, active.cursor, &overlays, &mut dirty);
        self.upload_instances(&layout, dirty);
        let effective_cursor_visible = if is_search_open { false } else { effective_visible };
        self.last_cursor_visible = effective_cursor_visible;
        let uniforms = self.frame_uniforms(&ctx, &active, active_pane, cursor, effective_cursor_visible);
        self.submit(frame, uniforms, active.defaults.1, layout.instance_count);
        if let (Some(capture), Some(start)) = (&mut self.frame_capture, capture_start) {
            let now = std::time::Instant::now();
            capture.record(now, now.duration_since(start).as_secs_f64() * 1000.0, acquire_ms);
        }
    }

    /// Force a full redraw when the inspector, preedit text or hovered link changed.
    fn note_overlay_inputs(&mut self, inspector_open: bool, preedit: &str, hovered_link: Option<(u64, &Hyperlink)>) {
        // The inspector's content changes without the grid changing.
        self.force_full |= inspector_open;
        if self.last_preedit != preedit {
            // Repaint the cells the previous composition covered.
            self.force_full = true;
            self.last_preedit.clear();
            self.last_preedit.push_str(preedit);
        }
        if self.last_hovered_link.as_ref().map(|(id, l)| (*id, l)) != hovered_link {
            self.force_full = true;
            self.last_hovered_link = hovered_link.map(|(id, l)| (id, l.clone()));
        }
    }

    /// Window geometry in cells and the instance-buffer layout derived from it.
    fn frame_layout(&self, panes: &[RenderPane], show_tabs: bool) -> FrameLayout {
        let top_rows = usize::from(show_tabs);
        let (cols, term_rows) = self.grid_size_for_tabs(show_tabs);
        let total_rows = term_rows + top_rows;
        let n = cols * total_rows;
        let extra_cells: usize = panes.iter().map(|p| p.rect.cols).sum();
        let scrollbar_start = n + extra_cells;
        let instance_count = scrollbar_start + panes.len();
        FrameLayout { cols, top_rows, total_rows, grid_cells: n, scrollbar_start, instance_count, pane_count: panes.len() }
    }

    /// The active pane's cursor, dirtiness, colors and progress, under one lock.
    fn read_active_pane(&self, pane: &RenderPane, top_rows: usize) -> ActivePane {
        let g = pane.grid.lock();
        let cur = g
            .cursor_pos()
            .map(|(cx, cy)| (pane.rect.col + g.cursor_visual_col(cy, cx), pane.rect.row + cy + top_rows));
        let dirty = g.all_dirty || g.viewport_dirty || g.dirty.iter().any(|&d| d);
        let cursor_color = g.cursor_color().map_or(self.config_cursor_color, |c| c | 1);
        let cursor_style = match g.modes.cursor_shape {
            1 | 2 => 0, // Block
            3 | 4 => 2, // Underline
            5 | 6 => 1, // Bar
            _ => self.cursor_style,
        };
        ActivePane { cursor: cur, dirty, defaults: (g.fg_color(), g.bg_color()), cursor_color, progress: g.progress, cursor_style }
    }

    /// Force a full redraw when progress, default colors or the active pane changed.
    fn note_active_changes(&mut self, active: &ActivePane, active_id: u64) {
        if active.progress != self.last_progress {
            self.last_progress = active.progress;
            self.force_full = true;
        }
        // The active pane's default colors go to the shader; other panes whose
        // defaults differ get explicit colors per cell (see `build_pane_row`).
        if active.defaults != self.last_defaults {
            self.last_defaults = active.defaults;
            self.force_full = true;
        }
        if active_id != self.last_active_pane {
            self.last_active_pane = active_id;
            self.force_full = true;
        }
    }

    /// True when no pane, cursor or buffer size changed since the last frame.
    fn nothing_changed(
        &self,
        panes: &[RenderPane],
        active_id: u64,
        active: &ActivePane,
        layout: &FrameLayout,
        cursor_visible: bool,
    ) -> bool {
        let any_dirty = active.dirty || panes.iter().filter(|p| p.id != active_id).any(|p| {
            let g = p.grid.lock();
            g.all_dirty || g.viewport_dirty || g.dirty.iter().any(|&d| d)
        });
        !any_dirty
            && self.instances.len() == layout.instance_count
            && active.cursor == self.last_cursor
            && cursor_visible == self.last_cursor_visible
    }

    /// The next swapchain texture, or `None` when the surface needs reconfiguring or failed.
    fn acquire_frame(&mut self) -> Option<wgpu::SurfaceTexture> {
        match self.surface.get_current_texture() {
            Ok(f) => Some(f),
            Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
                self.surface.configure(&self.device, &self.config);
                self.force_full = true;
                None
            }
            Err(e) => {
                eprintln!("litty: surface error: {e}");
                None
            }
        }
    }

    /// Resize the instance and viewport buffers to this frame's layout.
    fn ensure_buffers(&mut self, instance_count: usize, pane_count: usize) {
        if self.instances.len() != instance_count {
            self.instances.clear();
            self.instances.resize(instance_count, Instance::default());
            if self.instances.capacity() > instance_count.saturating_mul(2) && self.instances.capacity() > 4096 {
                self.instances.shrink_to_fit();
            }
            let needed_bytes = (instance_count * std::mem::size_of::<Instance>()) as u64;
            if self.instance_buf.size() < needed_bytes
                || (self.instance_buf.size() > needed_bytes.saturating_mul(2) && self.instance_buf.size() > 65536)
            {
                self.instance_buf = make_instance_buf(&self.device, instance_count);
            }
            self.force_full = true;
        }

        if self.viewport_buf.size() < (pane_count * std::mem::size_of::<Viewport>()) as u64 {
            self.viewport_buf = make_viewport_buffer(&self.device, pane_count);
            self.rebind_atlas();
        }
    }

    /// Build the dirty rows of every pane, its extra bottom row, scrollbar and image placements.
    fn collect_pane_instances(&mut self, panes: &[RenderPane], ctx: &FrameCtx, full: bool, dirty: &mut DirtyRows) {
        let FrameLayout { cols, top_rows, total_rows, grid_cells: n, scrollbar_start, .. } = ctx.layout;
        let defaults = ctx.defaults;
        let hovered_link = ctx.hovered_link;
        let active_rect = ctx.active_rect;
        let mut extra_start = n;
        self.placed.clear();
        let (cw, ch) = (self.atlas.cell_w, self.atlas.cell_h);

        for (pane_index, pane) in panes.iter().enumerate() {
            let pane_link = hovered_link.filter(|(id, _)| *id == pane.id).map(|(_, l)| l);
            let mut g = pane.grid.lock();
            let pane_full = full || g.all_dirty;
            let pane_defaults = Some((g.fg_color(), g.bg_color())).filter(|&d| d != defaults);
            let p_cols = pane.rect.cols.min(g.cols);
            let p_rows = pane.rect.rows.min(g.rows);
            let (right, top, bottom) = self.scrollbar_track(&pane.rect, ctx.show_tabs);
            let (history, offset) = g.scroll_position();
            let track_height = (bottom - top).max(0.0);
            let (thumb_y, height) = scrollbar_thumb(track_height, g.rows, history, offset);
            let y = top + thumb_y + (track_height - height).max(0.0) * g.scroll_fraction / history.max(1) as f64;
            self.viewports.push(Viewport {
                col: pane.rect.col as u32,
                row: (pane.rect.row + top_rows) as u32,
                cols: pane.rect.cols as u32,
                rows: pane.rect.rows as u32,
                extra_start: extra_start as u32,
                shift: -(g.scroll_fraction as f32) * ch as f32,
                top: self.cell_origin_px(pane.rect.row, pane.rect.col, ctx.show_tabs).1 as f32,
                bottom: self.cell_origin_px(pane.rect.row + pane.rect.rows, pane.rect.col, ctx.show_tabs).1 as f32,
                divider: u32::from(pane.rect.has_border_bottom) | (u32::from(pane.is_active || active_rect.is_some_and(|ar| {
                    ar.row == pane.rect.row + pane.rect.rows && ar.col < pane.rect.col + pane.rect.cols && ar.col + ar.cols > pane.rect.col
                })) << 1),
                _pad: 0,
            });
            self.instances[scrollbar_start + pane_index] = ui::scrollbar::instance(right, y, height, offset);

            // Collect visible images while g is locked to eliminate redundant grid locking in subsequent passes.
            g.visible_images(&mut self.image_draws);
            let (px0, py0) = self.cell_origin_px(pane.rect.row, pane.rect.col, ctx.show_tabs);
            let clip = [px0 as u32, py0 as u32, pane.rect.cols as u32 * cw, pane.rect.rows as u32 * ch];
            for draw in self.image_draws.drain(..) {
                let x = px0 as f32 + (draw.col as u32 * cw) as f32;
                let y = py0 as f32 + (draw.row as f32 - g.scroll_fraction as f32) * ch as f32;
                let rect = match draw.px {
                    Some([dx, dy, w, h]) => [x + dx, y + dy, w, h],
                    None if draw.fit => [x, y, (draw.cols as u32 * cw) as f32, (draw.rows as u32 * ch) as f32],
                    None => [x, y, draw.src[2], draw.src[3]],
                };
                self.placed.push(crate::render::image_layer::Placed { draw, rect, clip });
            }

            let row_ctx = PaneRowCtx {
                rect: &pane.rect,
                is_active: pane.is_active,
                active_rect: active_rect.as_ref(),
                hovered_link: pane_link,
                pane_defaults,
                p_cols,
                selection_colors: self.selection_colors,
            };
            let (mut cells, scratch) = self.row_target(cols);

            if g.scroll_fraction > 0.0 && (pane_full || g.dirty.iter().any(|&d| d)) {
                build_pane_row(&mut cells, scratch, &g, &row_ctx, p_rows, Some(extra_start), top_rows + pane.rect.row + p_rows);
            }
            extra_start += pane.rect.cols;
            g.viewport_dirty = false;
            if !pane_full && !g.dirty.iter().any(|&d| d) {
                continue;
            }

            for r in 0..p_rows {
                if pane_full || g.dirty[r] {
                    let dst_row = top_rows + pane.rect.row + r;
                    if dst_row < total_rows {
                        build_pane_row(&mut cells, scratch, &g, &row_ctx, r, None, dst_row);
                        dirty.mark(dst_row, dst_row);
                    }
                }
            }
            g.dirty.fill(false);
            g.all_dirty = false;
        }
    }

    /// Redraw everything after the atlas evicted glyphs (growing it first), up to three times.
    fn rebuild_after_eviction(&mut self, panes: &[RenderPane], ctx: &FrameCtx, dirty: &mut DirtyRows) {
        let FrameLayout { cols, top_rows, total_rows, .. } = ctx.layout;
        let defaults = ctx.defaults;
        let hovered_link = ctx.hovered_link;
        let tab_bar = ctx.tab_bar;
        let active_rect = ctx.active_rect;
            for _ in 0..3 {
                if self.atlas.grow_if_needed(&self.device, &self.queue) {
                    self.rebind_atlas();
                }
                if let Some(view) = tab_bar {
                    ui::tab_bar::build(&mut self.cells(cols), view);
                }
                for (pane_index, pane) in panes.iter().enumerate() {
                    let pane_link = hovered_link.filter(|(id, _)| *id == pane.id).map(|(_, l)| l);
                    let g = pane.grid.lock();
                    let pane_defaults = Some((g.fg_color(), g.bg_color())).filter(|&d| d != defaults);
                    let p_cols = pane.rect.cols.min(g.cols);
                    let p_rows = pane.rect.rows.min(g.rows);
                    let extra = self.viewports[pane_index].extra_start as usize;
                    let row_ctx = PaneRowCtx {
                        rect: &pane.rect,
                        is_active: pane.is_active,
                        active_rect: active_rect.as_ref(),
                        hovered_link: pane_link,
                        pane_defaults,
                        p_cols,
                        selection_colors: self.selection_colors,
                    };
                    let (mut cells, scratch) = self.row_target(cols);
                    if g.scroll_fraction > 0.0 {
                        build_pane_row(&mut cells, scratch, &g, &row_ctx, p_rows, Some(extra), top_rows + pane.rect.row + p_rows);
                    }
                    for r in 0..p_rows {
                        let dst_row = top_rows + pane.rect.row + r;
                        if dst_row < total_rows {
                            build_pane_row(&mut cells, scratch, &g, &row_ctx, r, None, dst_row);
                        }
                    }
                }
                if !self.atlas.take_evicted() {
                    break;
                }
            }
            if total_rows > 0 {
                dirty.lo = 0;
                dirty.hi = total_rows - 1;
            }
    }

    /// Link preview, IME preedit, find box, history popup and inspector; returns the cursor to draw.
    fn collect_overlays(
        &mut self,
        ctx: &FrameCtx,
        active_cur: Option<(usize, usize)>,
        overlays: &OverlayInputs,
        dirty: &mut DirtyRows,
    ) -> Option<(usize, usize)> {
        let FrameLayout { cols, top_rows, total_rows, .. } = ctx.layout;
        let hovered_link = ctx.hovered_link;
        let OverlayInputs { preedit, search, history_search, inspector } = *overlays;

        if let Some((_, link)) = hovered_link
            && total_rows > 0 && cols > 0 {
                ui::link_preview::build(&mut self.cells(cols), total_rows, link, dirty);
            }

        // IME composition (e.g. Telex / Pinyin preedit): underlined text at the cursor,
        // with the cursor moved to the end of the composition.
        let mut cursor = active_cur;
        if let (false, Some((cx, cy))) = (preedit.is_empty(), active_cur)
            && cols > 0 && cy < total_rows && cx < cols {
                cursor = Some(ui::preedit::build(&mut self.cells(cols), preedit, (cx, cy), dirty));
            }

        if search.is_open {
            ui::search_box::build(&mut self.cells(cols), top_rows, total_rows, search, dirty);
        }

        if let Some(search) = history_search
            && let Some(popup) = self.history_popup_geometry(&search) {
                ui::history_popup::build(&mut self.cells(cols), popup, total_rows, &search, dirty);
            }

        if let Some(lines) = inspector {
            ui::inspector::build(&mut self.cells(cols), top_rows, total_rows, lines, dirty);
        }

        cursor
    }

    /// Upload the dirty window rows and the extra rows / scrollbars after them.
    fn upload_instances(&self, layout: &FrameLayout, dirty: DirtyRows) {
        let FrameLayout { cols, total_rows, grid_cells: n, .. } = *layout;
        if total_rows > 0 && cols > 0 && dirty.lo <= dirty.hi {
            let a = dirty.lo * cols;
            let b = ((dirty.hi + 1) * cols).min(self.instances.len());
            if a < b {
                self.queue.write_buffer(
                    &self.instance_buf,
                    (a * std::mem::size_of::<Instance>()) as u64,
                    bytemuck::cast_slice(&self.instances[a..b]),
                );
            }
        }
        self.queue.write_buffer(
            &self.instance_buf,
            (n * std::mem::size_of::<Instance>()) as u64,
            bytemuck::cast_slice(&self.instances[n..]),
        );
    }

    /// The shader uniforms for this frame (layer 0; `submit` adjusts the layer).
    fn frame_uniforms(
        &self,
        ctx: &FrameCtx,
        active: &ActivePane,
        active_pane: &RenderPane,
        cursor: Option<(usize, usize)>,
        cursor_visible: bool,
    ) -> Uniforms {
        let FrameLayout { cols, top_rows, .. } = ctx.layout;
        let tab_bar = ctx.tab_bar;
        let defaults = ctx.defaults;
        let ActivePane { cursor_color, progress, cursor_style: active_cursor_style, .. } = *active;
        let effective_cursor_visible = cursor_visible;
        let (cx, cy) = cursor.unwrap_or((0, 0));
        let tab_info = match tab_bar {
            Some(view) => ((view.active as u32) << 16) | (view.titles.len() as u32 & 0xFFFF),
            None => 0,
        };
        let mut tab_shifts = [[0i32; 4]; MAX_SHIFTED_TABS / 4];
        let mut tab_drag = 0;
        if let Some(view) = tab_bar {
            for i in 0..view.titles.len().min(MAX_SHIFTED_TABS) {
                tab_shifts[i / 4][i % 4] = view.shift(i);
            }
            tab_drag = view.dragged.map_or(0, |d| d as u32 + 1);
        }
        let (underline_y, strike_y) = self.atlas.decoration_offsets();
        Uniforms {
            screen: [self.config.width as f32, self.config.height as f32],
            cell: [self.atlas.cell_w as f32, self.atlas.cell_h as f32],
            atlas_size: [self.atlas.size as f32; 2],
            offset: {
                let (ox, oy) = self.origin();
                [ox as f32, oy as f32]
            },
            cols: cols as u32,
            slots_per_row: self.atlas.slots_per_row,
            cursor: [cx as u32, cy as u32],
            cursor_visible: effective_cursor_visible as u32,
            cursor_style: active_cursor_style,
            default_fg: defaults.0,
            default_bg: defaults.1,
            tab_info,
            bg_opacity: self.bg_opacity,
            underline_y,
            strike_y,
            cursor_color,
            unfocused_dim: self.unfocused_dim,
            unfocused_fill: self.unfocused_fill,
            min_contrast: self.min_contrast,
            progress: progress.map_or(0, |(state, pct)| (state as u32) << 8 | pct.min(100) as u32),
            progress_row: (active_pane.rect.row + top_rows) as u32,
            progress_col: active_pane.rect.col as u32,
            progress_cols: active_pane.rect.cols as u32,
            bell_border: self.bell_border as u32,
            flip_y: self.postfx.is_some() as u32,
            layer: 0,
            pane_count: ctx.layout.pane_count as u32,
            tab_drag,
            tab_pad: [0; 3],
            tab_shifts,
        }
    }

    /// Upload viewports and uniforms, draw the cell / image passes and custom shaders, then present.
    fn submit(&mut self, frame: wgpu::SurfaceTexture, u: Uniforms, clear_bg: u32, instance_count: usize) {
        self.queue.write_buffer(&self.viewport_buf, 0, bytemuck::cast_slice(&self.viewports));

        // Lower z first; equal z keeps the placement order.
        self.placed.sort_by_key(|p| p.draw.z);
        // Images under the text split the cells into a background and a text layer.
        let layered = self.placed.first().is_some_and(|p| p.draw.z < 0);
        if layered {
            let text = Uniforms { layer: 2, ..u };
            self.queue.write_buffer(&self.text_uniform_buf, 0, bytemuck::bytes_of(&text));
        }
        let u = Uniforms { layer: layered as u32, ..u };
        self.queue.write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(&u));
        let flip = self.postfx.is_some();
        let target = (self.config.width, self.config.height);
        self.images.prepare(&self.device, &self.queue, &self.placed, target, flip);

        let screen = frame.texture.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        // With custom shaders the cells go to a texture the shaders read.
        let (width, height) = (self.config.width, self.config.height);
        let view = match &mut self.postfx {
            Some(fx) => fx.cell_target(&self.device, width, height).clone(),
            None => screen.clone(),
        };
        {
            let bg = clear_bg;
            let c = |s: u32| ((bg >> s) & 0xFF) as f64 / 255.0;
            let clear_a = if self.bg_opacity < 1.0 { self.bg_opacity as f64 } else { 1.0 };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cells"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: c(24), g: c(16), b: c(8), a: clear_a }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            let instances = 0..instance_count as u32;
            if layered {
                // Kitty z order: below cell backgrounds, below the text, above it.
                use crate::terminal::graphics::Z_BELOW_BG;
                self.images.draw(&mut pass, flip, target.1, i32::MIN..Z_BELOW_BG);
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_vertex_buffer(0, self.instance_buf.slice(..));
                pass.draw(0..4, instances.clone());
                self.images.draw(&mut pass, flip, target.1, Z_BELOW_BG..0);
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.text_bind_group, &[]);
                pass.set_vertex_buffer(0, self.instance_buf.slice(..));
                pass.draw(0..4, instances);
                self.images.draw(&mut pass, flip, target.1, 0..i32::MAX);
            } else {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_vertex_buffer(0, self.instance_buf.slice(..));
                pass.draw(0..4, instances);
                self.images.draw(&mut pass, flip, target.1, i32::MIN..i32::MAX);
            }
        }
        if let Some(fx) = &mut self.postfx {
            fx.run(&self.device, &self.queue, &mut encoder, &screen, self.mouse_px);
        }
        self.queue.submit(Some(encoder.finish()));
        frame.present();
    }
}
