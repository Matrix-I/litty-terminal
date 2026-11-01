use super::gpu::create_cells_pipeline;
use super::ui::scrollbar::SCROLLBAR_OVERLAY;

#[test]
#[ignore = "requires a Metal/Vulkan adapter; run explicitly for GPU pixel verification"]
fn precise_scroll_gpu_pixels() {
    use super::*;
    use wgpu::util::DeviceExt;
    let gpu = wgpu::Instance::default();
    let adapter = pollster::block_on(gpu.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("GPU adapter");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None)).unwrap();
    for flip_y in [0, 1] {
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let (layout, pipeline) = create_cells_pipeline(&device, format);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("scroll test"), size: wgpu::Extent3d { width: 64, height: 48, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let dummy = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("blank atlas"), size: wgpu::Extent3d { width: 16, height: 16, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[],
        });
        let dummy_view = dummy.create_view(&Default::default());
        let sampler = device.create_sampler(&Default::default());
        let uniforms = Uniforms { screen: [64.0, 48.0], cell: [16.0, 16.0], atlas_size: [16.0, 16.0],
            offset: [0.0, 8.0], cols: 4, slots_per_row: 1, bg_opacity: 1.0, pane_count: 2, flip_y, ..Default::default() };
        let uniform_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None, contents: bytemuck::bytes_of(&uniforms), usage: wgpu::BufferUsages::UNIFORM,
        });
        let viewports = [
            Viewport { cols: 3, rows: 2, extra_start: 8, shift: -12.0, top: 8.0, bottom: 40.0, divider: 1, ..Default::default() },
            Viewport { col: 3, cols: 1, rows: 2, extra_start: 11, top: 8.0, bottom: 40.0, ..Default::default() },
        ];
        let viewport_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None, contents: bytemuck::cast_slice(&viewports), usage: wgpu::BufferUsages::STORAGE,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &layout, entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: uniform_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&dummy_view) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&sampler) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&dummy_view) },
            wgpu::BindGroupEntry { binding: 4, resource: viewport_buf.as_entire_binding() },
        ] });
        let mut cells = Vec::new();
        for bg in [0xFF00_0080, 0x00FF_0080, 0x0000_FF80] {
            cells.extend([Instance { bg, ..Default::default() }; 4]);
        }
        cells.push(Instance { glyph: 54.0f32.to_bits(), fg: 12.0f32.to_bits(), bg: SCROLLBAR_OVERLAY | (24 << 8) });
        let instances = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None, contents: bytemuck::cast_slice(&cells), usage: wgpu::BufferUsages::VERTEX,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 256 * 48,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let mut encoder = device.create_command_encoder(&Default::default());
        let view = texture.create_view(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                ..Default::default()
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.set_vertex_buffer(0, instances.slice(..));
            pass.draw(0..4, 0..cells.len() as u32);
        }
        encoder.copy_texture_to_buffer(texture.as_image_copy(), wgpu::TexelCopyBufferInfo { buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(256), rows_per_image: None } },
            wgpu::Extent3d { width: 64, height: 48, depth_or_array_layers: 1 });
        queue.submit(Some(encoder.finish()));
        let (tx, rx) = std::sync::mpsc::channel();
        readback.slice(..).map_async(wgpu::MapMode::Read, move |result| { tx.send(result).unwrap(); });
        let _ = device.poll(wgpu::Maintain::Wait);
        rx.recv().unwrap().unwrap();
        let pixels = readback.slice(..).get_mapped_range();
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
            let mut png = png::Encoder::new(std::fs::File::create(path).unwrap(), 64, 48);
            png.set_color(png::ColorType::Rgba);
            png.set_depth(png::BitDepth::Eight);
            png.write_header().unwrap().write_image_data(&pixels).unwrap();
        }
    }
}

#[test]
fn test_shader_wgsl_syntax() {
    let shader_src = include_str!("../shader.wgsl");
    let module = wgpu::naga::front::wgsl::parse_str(shader_src).expect("Failed to parse WGSL");
    let mut validator = wgpu::naga::valid::Validator::new(
        wgpu::naga::valid::ValidationFlags::all(),
        wgpu::naga::valid::Capabilities::all(),
    );
    let info = validator.validate(&module);
    assert!(info.is_ok(), "Failed to validate WGSL module: {:?}", info.err());
}

#[test]
fn test_tab_bar_action_variants() {
    use super::TabBarAction;
    assert_eq!(TabBarAction::Select(0), TabBarAction::Select(0));
    assert_eq!(TabBarAction::CloseTab(1), TabBarAction::CloseTab(1));
    assert_ne!(TabBarAction::Select(0), TabBarAction::CloseTab(0));
    assert_eq!(TabBarAction::NewTab, TabBarAction::NewTab);
}
