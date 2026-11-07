use super::ui::scrollbar::SCROLLBAR_OVERLAY;
use crate::render::backend::{self, CommandEncoder, Texture, TextureFormat};

#[test]
#[ignore = "requires a Metal/Vulkan adapter; run explicitly for GPU pixel verification"]
fn precise_scroll_gpu_pixels() {
    use super::*;
    let (device, queue) = backend::shared_device_and_queue();
    for flip_y in [0, 1] {
        let format = TextureFormat::Rgba8Unorm;
        let pipelines = backend::shared_pipelines(&device, format);
        let texture = Texture::new(&device, 64, 48, format, true);
        let dummy = Texture::new(&device, 16, 16, TextureFormat::R8Unorm, false);
        let dummy_color = Texture::new(&device, 16, 16, TextureFormat::Rgba8Unorm, false);
        let sampler = device.nearest_sampler();
        let uniforms = Uniforms {
            screen: [64.0, 48.0],
            cell: [16.0, 16.0],
            atlas_size: [16.0, 16.0],
            offset: [0.0, 8.0],
            cols: 4,
            slots_per_row: 1,
            bg_opacity: 1.0,
            pane_count: 2,
            flip_y,
            ..Default::default()
        };
        let viewports = [
            Viewport { cols: 3, rows: 2, extra_start: 8, shift: -12.0, top: 8.0, bottom: 40.0, divider: 1, ..Default::default() },
            Viewport { col: 3, cols: 1, rows: 2, extra_start: 11, top: 8.0, bottom: 40.0, ..Default::default() },
        ];
        let viewport_buf = HostBuffer::from_slice(&device, &viewports);
        let mut cells = Vec::new();
        for bg in [0xFF00_0080, 0x00FF_0080, 0x0000_FF80] {
            cells.extend([Instance { bg, ..Default::default() }; 4]);
        }
        cells.push(Instance { glyph: 54.0f32.to_bits(), fg: 12.0f32.to_bits(), bg: SCROLLBAR_OVERLAY | (24 << 8) });
        let instances = HostBuffer::from_slice(&device, &cells);
        let mut encoder = CommandEncoder::new(&queue);
        {
            let mut pass = encoder.begin_render_pass(texture.raw_ptr(), Some([0.0, 0.0, 0.0, 1.0]));
            pass.draw_cells(
                &pipelines.cells,
                &instances,
                &uniforms,
                &viewport_buf,
                &dummy,
                &dummy_color,
                sampler,
                cells.len() as u32,
            );
        }
        encoder.submit_and_wait();
        let mut pixels = vec![0u8; 256 * 48];
        texture.read_pixels(&mut pixels, 256);
        let rgb = |x: usize, y: usize| {
            let y = if flip_y == 0 { y } else { 47 - y };
            pixels[y * 256 + x * 4..y * 256 + x * 4 + 3].to_vec()
        };
        assert_eq!(rgb(32, 7), [0, 0, 0]); // top clipping
        assert_eq!(rgb(32, 10), [255, 0, 0]);
        assert_eq!(rgb(32, 20), [0, 255, 0]);
        assert_eq!(rgb(32, 35), [0, 0, 255]); // additional bottom row
        assert_eq!(rgb(32, 39), [53, 54, 64]); // split divider stays fixed
        assert_eq!(rgb(32, 40), [0, 0, 0]); // bottom clipping
        assert_eq!(rgb(53, 20), [255, 0, 0]); // adjacent pane is stationary; exactly 10px width
        assert_ne!(rgb(63, 24), [0, 255, 0]); // flush against right edge
        assert_eq!(rgb(54, 12), [255, 0, 0]); // rounded top-left corner
        if flip_y == 0 && let Some(path) = std::env::var_os("LITTY_SCROLL_TEST_IMAGE") {
            let data = crate::terminal::graphics::png::encode_test_png(64, 48, 6, &pixels);
            std::fs::write(path, data).unwrap();
        }
    }
}

#[test]
fn test_shader_compilation_and_pipelines() {
    let (device, _queue) = backend::shared_device_and_queue();
    let _pipes_bgra = backend::shared_pipelines(&device, TextureFormat::Bgra8Unorm);
    let _pipes_rgba = backend::shared_pipelines(&device, TextureFormat::Rgba8Unorm);
}

#[test]
fn test_uniforms_layout_matches_shader() {
    assert_eq!(std::mem::size_of::<super::Uniforms>(), 208);
    assert_eq!(std::mem::size_of::<super::Viewport>(), 40);
    assert_eq!(std::mem::size_of::<super::Instance>(), 12);
    assert_eq!(std::mem::offset_of!(super::Uniforms, tab_shifts), 144, "uniform arrays need 16-byte alignment");
}

#[test]
fn test_tab_bar_action_variants() {
    use super::TabBarAction;
    assert_eq!(TabBarAction::Select(0), TabBarAction::Select(0));
    assert_eq!(TabBarAction::CloseTab(1), TabBarAction::CloseTab(1));
    assert_ne!(TabBarAction::Select(0), TabBarAction::CloseTab(0));
    assert_eq!(TabBarAction::NewTab, TabBarAction::NewTab);
}
