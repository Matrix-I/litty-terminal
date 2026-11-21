use super::ui::scrollbar::SCROLLBAR_OVERLAY;
use crate::render::backend::{self, CommandEncoder, Texture, TextureFormat};

#[test]
#[ignore = "requires a Metal/Vulkan adapter; run explicitly for GPU pixel verification"]
fn precise_scroll_gpu_pixels() {
    let _gpu = crate::render::backend::TEST_GPU.lock();
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

/// Two empty default-background cells, then two with an explicit red
/// background, drawn as `Renderer::submit` draws a frame at
/// `background-opacity` 0.5 over a #203040 default background, then its
/// `output_alpha` passes (`premultiply`, `corners`; none when both are
/// false). Returns the RGBA rows, top first; `None` without a GPU.
fn render_translucent_frame(premultiply: bool, corners: bool) -> Option<Vec<u8>> {
    let _gpu = crate::render::backend::TEST_GPU.lock();
    use super::*;
    let (device, queue) = backend::shared_device_and_queue();
    if !device.has_gpu() {
        eprintln!("no GPU; skipping");
        return None;
    }
    let format = TextureFormat::Rgba8Unorm;
    let pipelines = backend::shared_pipelines(&device, format);
    let texture = Texture::new(&device, 64, 16, format, true);
    let dummy = Texture::new(&device, 16, 16, TextureFormat::R8Unorm, false);
    let dummy_color = Texture::new(&device, 16, 16, TextureFormat::Rgba8Unorm, false);
    let sampler = device.nearest_sampler();
    let opacity = 0.5;
    let uniforms = Uniforms {
        screen: [64.0, 16.0],
        cell: [16.0, 16.0],
        atlas_size: [16.0, 16.0],
        cols: 4,
        slots_per_row: 1,
        bg_opacity: opacity,
        default_bg: 0x2030_4000,
        pane_count: 1,
        // As `Renderer::submit` passes `corner_radius` to the corner shader.
        tab_pad: [0, 6.0f32.to_bits(), 0],
        ..Default::default()
    };
    // No scroll-in row: `extra_start` past the cells.
    let viewports = [Viewport { cols: 4, rows: 1, extra_start: 4, bottom: 16.0, ..Default::default() }];
    let viewport_buf = HostBuffer::from_slice(&device, &viewports);
    let explicit_red = Instance { bg: 0xFF00_0080, ..Default::default() };
    let cells = [Instance::default(), Instance::default(), explicit_red, explicit_red];
    let instances = HostBuffer::from_slice(&device, &cells);
    let mut encoder = CommandEncoder::new(&queue);
    {
        // As `Renderer::submit` clears a translucent frame.
        let clear = [0x20 as f64 / 255.0, 0x30 as f64 / 255.0, 0x40 as f64 / 255.0, opacity as f64];
        let mut pass = encoder.begin_render_pass(texture.raw_ptr(), Some(clear));
        pass.draw_cells(&pipelines.cells, &instances, &uniforms, &viewport_buf, &dummy, &dummy_color, sampler, 4);
    }
    if premultiply || corners {
        let mut pass = encoder.begin_render_pass(texture.raw_ptr(), None);
        super::frame::output_alpha(&mut pass, &pipelines, &uniforms, premultiply, corners);
    }
    encoder.submit_and_wait();
    let mut pixels = vec![0u8; 256 * 16];
    texture.read_pixels(&mut pixels, 256);
    Some(pixels)
}

/// RGBA of pixel (`x`, `y`) of a 64-pixel-wide frame from `render_translucent_frame`.
fn rgba(frame: &[u8], x: usize, y: usize) -> &[u8] {
    &frame[y * 256 + x * 4..][..4]
}

fn near(got: &[u8], want: [u8; 4]) -> bool {
    got.iter().zip(want).all(|(&g, w)| g.abs_diff(w) <= 1)
}

/// `background-opacity`: the frame is cleared to the default background at
/// that opacity, and empty default-background cells drawn over it must keep
/// it. Blending their alpha over the clear's stacked it (0.5 came out 0.75).
#[test]
fn default_background_cells_keep_the_configured_opacity() {
    let Some(frame) = render_translucent_frame(false, false) else { return };
    let alpha = rgba(&frame, 8, 8)[3];
    assert!(alpha.abs_diff(128) <= 1, "default background alpha {alpha}, expected about 128");
    assert_eq!(rgba(&frame, 8, 8)[..3], [0x20, 0x30, 0x40], "default background color");
    assert_eq!(rgba(&frame, 40, 8), [255, 0, 0, 255], "an explicit background stays opaque");
}

/// Compositors (premultiplied Vulkan surfaces, non-opaque `CAMetalLayer`s)
/// read the frame as premultiplied: a straight #203040 at half opacity
/// showed as #203040 plus half the desktop, brighter than configured.
/// Only a translucent frame on a compositor that reads premultiplied alpha
/// is premultiplied; straight-alpha compositors would show it too dark.
#[test]
fn only_a_translucent_frame_for_a_premultiplied_compositor_is_premultiplied() {
    use super::frame::needs_premultiply;
    assert!(needs_premultiply(0.5, true));
    assert!(!needs_premultiply(0.5, false), "straight-alpha compositor");
    assert!(!needs_premultiply(1.0, true), "opaque frame");
    assert!(!needs_premultiply(1.0, false));
}

#[test]
fn a_translucent_frame_is_presented_premultiplied() {
    let Some(frame) = render_translucent_frame(true, false) else { return };
    assert!(near(rgba(&frame, 8, 8), [0x10, 0x18, 0x20, 128]), "default background {:?}", rgba(&frame, 8, 8));
    assert_eq!(rgba(&frame, 40, 8), [255, 0, 0, 255], "opaque pixels are unchanged");
}

/// Corners fade after the premultiply, so every pixel of the default
/// background, the partly cut anti-aliased corner edge included, is the
/// background color times its alpha. Fading first would scale that edge's
/// color by the cut twice.
#[test]
fn rounded_corners_fade_a_premultiplied_frame() {
    let Some(frame) = render_translucent_frame(true, true) else { return };
    assert_eq!(rgba(&frame, 0, 0), [0, 0, 0, 0], "outside the rounded corner");
    let mut edge = 0;
    // The two default-background cells: x 0..32.
    for y in 0..16 {
        for x in 0..32 {
            let p = rgba(&frame, x, y);
            let a = u32::from(p[3]);
            for (c, straight) in p[..3].iter().zip([0x20u32, 0x30, 0x40]) {
                let want = (straight * a + 127) / 255;
                assert!(u32::from(*c).abs_diff(want) <= 1, "({x}, {y}) {p:?} is not premultiplied");
            }
            edge += usize::from(a > 0 && a < 128);
        }
    }
    assert!(edge > 0, "the corner has a partly cut edge to check");
}

#[test]
fn test_shader_compilation_and_pipelines() {
    let _gpu = crate::render::backend::TEST_GPU.lock();
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

/// Tab bar geometry of an 80-column window with 10x20 px cells and no title bar band (macOS).
fn strip_only(tabs: usize) -> super::TabBarHit {
    super::TabBarHit { titlebar: false, tabs, bar_h: 20.0 * super::TAB_BAR_CELLS, strip_top: 0.0, cols: 80 }
}

#[test]
fn a_lone_tab_has_no_strip_to_click() {
    // macOS with one tab draws no strip: the first terminal row starts at the top
    // of the window, so its clicks (double-click to select, a click near the
    // traffic lights) must reach the terminal, not select or close the tab.
    let hit = strip_only(1);
    assert_eq!(hit.action(30, 10.0), None, "double-click on the first row");
    assert_eq!(hit.action(1, 10.0), None, "click under the close button must not close the tab");
}

#[test]
fn two_tabs_keep_their_strip_actions() {
    let hit = strip_only(2);
    assert_eq!(hit.action(1, 10.0), Some(super::TabBarAction::CloseTab(0)));
    assert_eq!(hit.action(50, 10.0), Some(super::TabBarAction::Select(1)));
    assert_eq!(hit.action(79, 10.0), Some(super::TabBarAction::NewTab));
    assert_eq!(hit.action(50, 40.0), None, "below the strip");
}

#[test]
fn the_title_bar_band_stays_clickable_with_a_lone_tab() {
    let hit = super::TabBarHit { titlebar: true, ..strip_only(1) };
    assert_eq!(hit.action(30, 10.0), Some(super::TabBarAction::TitleBar));
    assert_eq!(hit.action(79, 10.0), Some(super::TabBarAction::Control(super::WindowControl::Close)));
    assert_eq!(hit.action(30, 50.0), None, "no strip under the title bar with one tab");
}
