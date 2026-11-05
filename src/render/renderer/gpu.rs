//! wgpu objects shared by every window: the instance, one adapter / device /
//! queue, the cached cell pipeline per surface format, and buffer / bind-group
//! constructors.

use std::sync::Arc;

use super::{Instance, Viewport};
use crate::render::atlas::Atlas;

pub(super) fn make_viewport_buffer(device: &wgpu::Device, panes: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("pane viewports"),
        size: (panes.max(1) * std::mem::size_of::<Viewport>()) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

static WGPU_INSTANCE: std::sync::OnceLock<wgpu::Instance> = std::sync::OnceLock::new();

pub(super) fn global_instance() -> &'static wgpu::Instance {
    WGPU_INSTANCE.get_or_init(|| {
        #[cfg(target_os = "macos")]
        let backends = wgpu::Backends::METAL;
        #[cfg(not(target_os = "macos"))]
        let backends = wgpu::Backends::VULKAN;

        wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends,
            flags: wgpu::InstanceFlags::empty(),
            ..Default::default()
        })
    })
}

struct SharedGpu {
    adapter: wgpu::Adapter,
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
}

static SHARED_GPU: parking_lot::Mutex<Option<SharedGpu>> = parking_lot::const_mutex(None);

static SHARED_PIPELINE: parking_lot::Mutex<Option<(wgpu::TextureFormat, wgpu::BindGroupLayout, wgpu::RenderPipeline)>> =
    parking_lot::const_mutex(None);

/// Pass that clears the outside of the window's rounded corners to transparent.
pub(super) fn create_corner_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    bgl: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::include_wgsl!("../shaders/shader.wgsl"));
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("corner layout"),
        bind_group_layouts: &[bgl],
        push_constant_ranges: &[],
    });
    // dst * (1 - src.a): the cut area fades out, antialiased on the arc.
    let fade = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::Zero,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("corners"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_corner"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_corner"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState { color: fade, alpha: fade }),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview: None,
        cache: None,
    })
}

pub(super) fn create_cells_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
) -> (wgpu::BindGroupLayout, wgpu::RenderPipeline) {
    let shader = device.create_shader_module(wgpu::include_wgsl!("../shaders/shader.wgsl"));
    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
        ],
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("layout"),
        bind_group_layouts: &[&bgl],
        push_constant_ranges: &[],
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("cells"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Instance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &wgpu::vertex_attr_array![0 => Uint32, 1 => Uint32, 2 => Uint32],
            }],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview: None,
        cache: None,
    });
    (bgl, pipeline)
}

pub(super) fn make_bind_group(
    device: &wgpu::Device,
    bgl: &wgpu::BindGroupLayout,
    uniform_buf: &wgpu::Buffer,
    atlas: &Atlas,
    sampler: &wgpu::Sampler,
    viewport_buf: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("bind group"),
        layout: bgl,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: uniform_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 4, resource: viewport_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&atlas.view) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&atlas.color_view) },
        ],
    })
}

pub(super) fn make_instance_buf(device: &wgpu::Device, cells: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("instances"),
        size: (cells.max(1) * std::mem::size_of::<Instance>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// The process-wide adapter, device and queue; created on the first window.
pub(super) fn shared_device(surface: &wgpu::Surface<'static>) -> (wgpu::Adapter, Arc<wgpu::Device>, Arc<wgpu::Queue>) {
    let instance = global_instance();
    let mut guard = SHARED_GPU.lock();
    if let Some(gpu) = guard.as_ref() {
        (gpu.adapter.clone(), gpu.device.clone(), gpu.queue.clone())
    } else {
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(surface),
            force_fallback_adapter: false,
        }))
        .expect("no suitable GPU adapter (Metal/Vulkan)");
        let (dev, q) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("litty"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
                memory_hints: wgpu::MemoryHints::MemoryUsage,
            },
            None,
        ))
        .expect("request device");
        let dev = Arc::new(dev);
        let q = Arc::new(q);
        *guard = Some(SharedGpu {
            adapter: adapter.clone(),
            device: dev.clone(),
            queue: q.clone(),
        });
        (adapter, dev, q)
    }
}

/// The cell pipeline for `format`, reused while every window has the same format.
pub(super) fn shared_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
) -> (wgpu::BindGroupLayout, wgpu::RenderPipeline) {
    let mut guard = SHARED_PIPELINE.lock();
    if let Some((cached_fmt, ref cached_bgl, ref cached_pipe)) = *guard {
        if cached_fmt == format {
            (cached_bgl.clone(), cached_pipe.clone())
        } else {
            let (b, p) = create_cells_pipeline(device, format);
            *guard = Some((format, b.clone(), p.clone()));
            (b, p)
        }
    } else {
        let (b, p) = create_cells_pipeline(device, format);
        *guard = Some((format, b.clone(), p.clone()));
        (b, p)
    }
}
