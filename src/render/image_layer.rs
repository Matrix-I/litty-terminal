//! GPU side of inline images: one texture per image (uploaded once, dropped
//! when unused for a while) and one textured quad per visible placement,
//! drawn in the cell render pass below or above the cells by z.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::render::backend::{self, Device, HostBuffer, ObjPtr, Queue, RenderPass, Texture, TextureFormat};
use crate::terminal::graphics::ImageDraw;

/// Frame count without being rendered before an unplaced texture is freed during active rendering.
const KEEP_FRAMES: u64 = 60;
/// Maximum duration an unplaced texture is retained in GPU VRAM before eager eviction.
const KEEP_DURATION: Duration = Duration::from_secs(5);

#[repr(C)]
#[derive(Copy, Clone, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct Quad {
    /// x, y, width, height in px.
    rect: [f32; 4],
    /// u0, v0, u1, v1.
    uv: [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    screen: [f32; 2],
    flip_y: u32,
    _pad: u32,
}

/// One placement to draw: the image, where (px) and the pane it is clipped to.
pub struct Placed {
    pub draw: ImageDraw,
    pub rect: [f32; 4],
    pub clip: [u32; 4],
}

struct TextureEntry {
    texture: Texture,
    last_frame: u64,
    last_seen: Instant,
}

pub struct ImageLayer {
    pipeline: ObjPtr,
    uniforms: Uniforms,
    quads: HostBuffer<Quad>,
    quad_data: Vec<Quad>,
    /// Image key → GPU texture, last frame, and timestamp.
    textures: HashMap<u64, TextureEntry>,
    frame: u64,
    /// This frame's draws: image key, scissor rectangle and z, in quad order.
    batch: Vec<(u64, [u32; 4], i32)>,
}

impl ImageLayer {
    pub fn new(device: &Device, format: TextureFormat) -> Self {
        let pipes = backend::shared_pipelines(device, format);
        ImageLayer {
            pipeline: pipes.image,
            uniforms: Uniforms::default(),
            quads: HostBuffer::new(device, 16),
            quad_data: Vec::with_capacity(16),
            textures: HashMap::new(),
            frame: 0,
            batch: Vec::new(),
        }
    }

    /// Discard all cached GPU textures and batches (e.g. when shrinking drawables or idle in Dock).
    pub fn clear(&mut self) {
        self.textures.clear();
        self.batch.clear();
    }

    /// Upload new images and this frame's quads. `screen` is the target size.
    pub fn prepare(&mut self, device: &Device, queue: &Queue, placed: &[Placed], screen: (u32, u32), flip: bool) {
        self.frame += 1;
        self.batch.clear();
        self.quad_data.clear();
        let now = Instant::now();
        if placed.is_empty() {
            self.textures.retain(|_, entry| {
                self.frame.saturating_sub(entry.last_frame) < KEEP_FRAMES
                    && now.saturating_duration_since(entry.last_seen) < KEEP_DURATION
            });
            return;
        }
        self.uniforms = Uniforms { screen: [screen.0 as f32, screen.1 as f32], flip_y: flip as u32, _pad: 0 };
        for p in placed {
            let d = &p.draw;
            if !self.textures.contains_key(&d.key) {
                let texture = self.upload(device, queue, d);
                self.textures.insert(d.key, TextureEntry { texture, last_frame: self.frame, last_seen: now });
            }
            if let Some(entry) = self.textures.get_mut(&d.key) {
                entry.last_frame = self.frame;
                entry.last_seen = now;
            }
            push_quad(p, screen, &mut self.quad_data, &mut self.batch);
        }
        self.quads.write(device, &self.quad_data);
        self.textures.retain(|_, entry| {
            entry.last_frame == self.frame
                || (self.frame.saturating_sub(entry.last_frame) < KEEP_FRAMES
                    && now.saturating_duration_since(entry.last_seen) < KEEP_DURATION)
        });
    }

    /// Prune any textures not rendered recently, freeing GPU VRAM on idle terminals.
    /// Returns the deadline for the next texture eviction, if any unplaced textures remain.
    pub fn prune_idle(&mut self, now: Instant) -> Option<Instant> {
        self.textures.retain(|_, entry| now.saturating_duration_since(entry.last_seen) < KEEP_DURATION);
        self.textures.values().map(|entry| entry.last_seen + KEEP_DURATION).min()
    }

    fn upload(&self, device: &Device, queue: &Queue, d: &ImageDraw) -> Texture {
        let texture = Texture::new(device, d.width, d.height, TextureFormat::Rgba8Unorm, false);
        texture.write_rect(queue, 0, 0, d.width, d.height, &d.rgba, 4 * d.width);
        texture
    }

    /// Draw the prepared quads whose z is in `z` (inside the cell render pass).
    pub fn draw(&self, pass: &mut RenderPass, device: &Device, flip: bool, target: (u32, u32), z: std::ops::Range<i32>) {
        if !self.batch.iter().any(|b| z.contains(&b.2)) {
            return;
        }
        let sampler = device.linear_sampler();
        for (i, (key, clip, zi)) in self.batch.iter().enumerate() {
            let Some(entry) = self.textures.get(key) else { continue };
            if !z.contains(zi) {
                continue;
            }
            let Some([sx, sy, sw, sh]) = compute_scissor_rect(*clip, flip, target.1) else {
                continue;
            };
            pass.set_scissor_rect(sx, sy, sw, sh);
            pass.draw_image_quad(&self.pipeline, &self.quads, &self.uniforms, &entry.texture, sampler, i as u32);
        }
        // Restore full-target scissor rect for subsequent cell passes.
        pass.set_scissor_rect(0, 0, target.0.max(1), target.1.max(1));
    }
}

pub(crate) fn compute_scissor_rect([x, y, w, h]: [u32; 4], flip: bool, height: u32) -> Option<[u32; 4]> {
    if w == 0 || h == 0 {
        return None;
    }
    let clamped_h = h.min(height.saturating_sub(y));
    if clamped_h == 0 {
        return None;
    }
    let scissor_y = if flip {
        height.checked_sub(y + clamped_h)?
    } else {
        y
    };
    if scissor_y.saturating_add(clamped_h) > height {
        return None;
    }
    Some([x, scissor_y, w, clamped_h])
}

/// CPU geometry preparation shares the retained frame buffers.
fn push_quad(p: &Placed, screen: (u32, u32), quads: &mut Vec<Quad>, batch: &mut Vec<(u64, [u32; 4], i32)>) {
    let d = &p.draw;
    let (w, h) = (d.width as f32, d.height as f32);
    let [sx, sy, sw, sh] = d.src;
    quads.push(Quad { rect: p.rect, uv: [sx / w, sy / h, (sx + sw) / w, (sy + sh) / h] });
    // Clip to the pane and the target.
    let [cx, cy, cw, ch] = p.clip;
    let (x, y) = (cx.min(screen.0), cy.min(screen.1));
    let clip = [x, y, cw.min(screen.0 - x), ch.min(screen.1 - y)];
    batch.push((d.key, clip, d.z));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_geometry_reuses_frame_buffers_after_warmup() {
        let placed = Placed {
            draw: ImageDraw {
                key: 1, width: 10, height: 10, rgba: vec![0; 400].into(),
                src: [0.0, 0.0, 10.0, 10.0], row: 0, col: 0, cols: 1, rows: 1,
                fit: false, z: 0, px: None,
            },
            rect: [0.0, 0.0, 10.0, 10.0], clip: [90, 95, 30, 30],
        };
        let mut quads = Vec::new();
        let mut batch = Vec::new();
        for count in [1, 4, 16] {
            quads.clear();
            batch.clear();
            for _ in 0..count { push_quad(&placed, (100, 100), &mut quads, &mut batch); }
            let allocations = crate::test_alloc::allocations(|| {
                for _ in 0..1000 {
                    quads.clear();
                    batch.clear();
                    for _ in 0..count { push_quad(&placed, (100, 100), &mut quads, &mut batch); }
                }
            });
            assert_eq!(allocations, 0, "steady CPU image geometry");
            assert_eq!(quads.len(), count);
            assert_eq!(quads[0].uv, [0.0, 0.0, 1.0, 1.0]);
            assert_eq!(batch[0].1, [90, 95, 10, 5]);
        }
    }

    #[test]
    fn test_compute_scissor_rect_flipped_underflow_prevention() {
        // Normal in-bounds
        assert_eq!(compute_scissor_rect([10, 20, 30, 40], true, 100), Some([10, 40, 30, 40]));
        assert_eq!(compute_scissor_rect([10, 20, 30, 40], false, 100), Some([10, 20, 30, 40]));

        // Rect partially exceeds target height (would underflow in naive height - y - h)
        assert_eq!(compute_scissor_rect([10, 80, 30, 50], true, 100), Some([10, 0, 30, 20]));
        assert_eq!(compute_scissor_rect([10, 80, 30, 50], false, 100), Some([10, 80, 30, 20]));

        // Completely outside target
        assert_eq!(compute_scissor_rect([10, 100, 30, 50], true, 100), None);
        assert_eq!(compute_scissor_rect([10, 150, 30, 50], true, 100), None);

        // Zero target height or zero rect dimensions
        assert_eq!(compute_scissor_rect([10, 0, 30, 50], true, 0), None);
        assert_eq!(compute_scissor_rect([10, 20, 0, 40], true, 100), None);
        assert_eq!(compute_scissor_rect([10, 20, 30, 0], true, 100), None);
    }

    #[test]
    fn test_image_layer_prune_idle() {
        let (device, queue) = backend::shared_device_and_queue();
        let mut layer = ImageLayer::new(&device, TextureFormat::Bgra8Unorm);

        let draw = ImageDraw {
            key: 42,
            width: 2,
            height: 2,
            rgba: vec![255; 16].into(),
            src: [0.0, 0.0, 2.0, 2.0],
            row: 0,
            col: 0,
            cols: 1,
            rows: 1,
            fit: false,
            z: 0,
            px: None,
        };
        let placed = Placed {
            draw,
            rect: [0.0, 0.0, 10.0, 10.0],
            clip: [0, 0, 100, 100],
        };

        // Prepare once: texture uploaded
        layer.prepare(&device, &queue, &[placed], (100, 100), false);
        assert_eq!(layer.textures.len(), 1);

        let now = Instant::now();
        // Prune immediately: still fresh (< 5s), returns deadline for 5s in the future
        let next = layer.prune_idle(now);
        assert_eq!(layer.textures.len(), 1);
        assert!(next.is_some());

        // Prune 6 seconds later without rendering: aged past KEEP_DURATION, texture evicted
        let future = now + Duration::from_secs(6);
        let next_future = layer.prune_idle(future);
        assert_eq!(layer.textures.len(), 0, "idle texture must be evicted past KEEP_DURATION");
        assert!(next_future.is_none());
    }
}
