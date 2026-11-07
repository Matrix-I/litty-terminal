//! Process-wide Metal device, command queue, shader library, and cached render pipeline states.

use std::sync::Arc;

use super::ffi::*;
use crate::render::backend::TextureFormat;

/// Process-wide Metal device and compiled shader library.
#[derive(Clone)]
pub struct Device {
    pub(super) raw: ObjPtr,
    pub(super) library: ObjPtr,
    pub(super) nearest_sampler: ObjPtr,
    pub(super) linear_sampler: ObjPtr,
}

/// Process-wide Metal command queue.
#[derive(Clone)]
pub struct Queue {
    pub(super) raw: ObjPtr,
}

struct SharedMetal {
    device: Arc<Device>,
    queue: Arc<Queue>,
}

static SHARED_METAL: parking_lot::Mutex<Option<SharedMetal>> = parking_lot::const_mutex(None);

static SHARED_PIPELINES: parking_lot::Mutex<Option<(TextureFormat, RenderPipelines)>> =
    parking_lot::const_mutex(None);

#[derive(Clone)]
pub struct RenderPipelines {
    pub cells: ObjPtr,
    pub corner: ObjPtr,
    pub image: ObjPtr,
}

pub fn shared_device_and_queue() -> (Arc<Device>, Arc<Queue>) {
    let mut guard = SHARED_METAL.lock();
    if let Some(m) = guard.as_ref() {
        return (m.device.clone(), m.queue.clone());
    }
    // SAFETY: MTLCreateSystemDefaultDevice returns a retained MTLDevice on macOS.
    let (dev, q) = unsafe {
        let raw_dev = MTLCreateSystemDefaultDevice();
        assert!(!raw_dev.is_null(), "MTLCreateSystemDefaultDevice failed: no Metal GPU");
        let raw_queue = msg_send!(raw_dev, c"newCommandQueue"; -> Id);
        assert!(!raw_queue.is_null(), "newCommandQueue failed");

        let shader_src = include_str!("../../shaders/shader.metal");
        let src_ns = ns_string(shader_src).expect("shader NSString");
        let mut err: Id = std::ptr::null_mut();
        let lib = msg_send!(
            raw_dev,
            c"newLibraryWithSource:options:error:",
            src_ns => Id,
            std::ptr::null_mut() => Id,
            &mut err as *mut Id => *mut Id;
            -> Id
        );
        if lib.is_null() {
            panic!("failed to compile shader.metal: {}", ns_error_string(err));
        }

        let nearest = create_sampler(raw_dev, MTL_SAMPLER_MIN_MAG_FILTER_NEAREST);
        let linear = create_sampler(raw_dev, MTL_SAMPLER_MIN_MAG_FILTER_LINEAR);

        (
            Arc::new(Device {
                raw: ObjPtr(raw_dev),
                library: ObjPtr(lib),
                nearest_sampler: nearest,
                linear_sampler: linear,
            }),
            Arc::new(Queue { raw: ObjPtr(raw_queue) }),
        )
    };
    *guard = Some(SharedMetal { device: dev.clone(), queue: q.clone() });
    (dev, q)
}

unsafe fn create_sampler(raw_dev: Id, filter: usize) -> ObjPtr {
    unsafe {
        let cls = class(c"MTLSamplerDescriptor").expect("MTLSamplerDescriptor");
        let desc = msg_send!(cls, c"new"; -> Id);
        msg_send!(desc, c"setMinFilter:", filter => usize);
        msg_send!(desc, c"setMagFilter:", filter => usize);
        let samp = msg_send!(raw_dev, c"newSamplerStateWithDescriptor:", desc => Id; -> Id);
        msg_send!(desc, c"release");
        assert!(!samp.is_null(), "newSamplerStateWithDescriptor failed");
        ObjPtr(samp)
    }
}

#[derive(Copy, Clone)]
pub(super) enum BlendMode {
    None,
    Alpha,
    CornerFade,
}

pub(super) unsafe fn build_pipeline(
    raw_dev: Id,
    lib: Id,
    vs_name: &str,
    fs_name: &str,
    format: TextureFormat,
    blend: BlendMode,
) -> Result<ObjPtr, String> {
    unsafe {
        let vs_ns = ns_string(vs_name).ok_or("invalid vs name")?;
        let fs_ns = ns_string(fs_name).ok_or("invalid fs name")?;
        let vs_fn = msg_send!(lib, c"newFunctionWithName:", vs_ns => Id; -> Id);
        if vs_fn.is_null() {
            return Err(format!("vertex function `{vs_name}` not found"));
        }
        let fs_fn = msg_send!(lib, c"newFunctionWithName:", fs_ns => Id; -> Id);
        if fs_fn.is_null() {
            msg_send!(vs_fn, c"release");
            return Err(format!("fragment function `{fs_name}` not found"));
        }

        let cls = class(c"MTLRenderPipelineDescriptor").ok_or("MTLRenderPipelineDescriptor")?;
        let desc = msg_send!(cls, c"new"; -> Id);
        msg_send!(desc, c"setVertexFunction:", vs_fn => Id);
        msg_send!(desc, c"setFragmentFunction:", fs_fn => Id);

        let attachments = msg_send!(desc, c"colorAttachments"; -> Id);
        let att0 = msg_send!(attachments, c"objectAtIndexedSubscript:", 0usize => usize; -> Id);
        msg_send!(att0, c"setPixelFormat:", mtl_pixel_format(format) => usize);

        match blend {
            BlendMode::None => {
                msg_send!(att0, c"setBlendingEnabled:", false => bool);
            }
            BlendMode::Alpha => {
                msg_send!(att0, c"setBlendingEnabled:", true => bool);
                msg_send!(att0, c"setSourceRGBBlendFactor:", MTL_BLEND_FACTOR_SRC_ALPHA => usize);
                msg_send!(att0, c"setDestinationRGBBlendFactor:", MTL_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA => usize);
                msg_send!(att0, c"setRgbBlendOperation:", MTL_BLEND_OPERATION_ADD => usize);
                msg_send!(att0, c"setSourceAlphaBlendFactor:", MTL_BLEND_FACTOR_ONE => usize);
                msg_send!(att0, c"setDestinationAlphaBlendFactor:", MTL_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA => usize);
                msg_send!(att0, c"setAlphaBlendOperation:", MTL_BLEND_OPERATION_ADD => usize);
            }
            BlendMode::CornerFade => {
                msg_send!(att0, c"setBlendingEnabled:", true => bool);
                msg_send!(att0, c"setSourceRGBBlendFactor:", MTL_BLEND_FACTOR_ZERO => usize);
                msg_send!(att0, c"setDestinationRGBBlendFactor:", MTL_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA => usize);
                msg_send!(att0, c"setRgbBlendOperation:", MTL_BLEND_OPERATION_ADD => usize);
                msg_send!(att0, c"setSourceAlphaBlendFactor:", MTL_BLEND_FACTOR_ZERO => usize);
                msg_send!(att0, c"setDestinationAlphaBlendFactor:", MTL_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA => usize);
                msg_send!(att0, c"setAlphaBlendOperation:", MTL_BLEND_OPERATION_ADD => usize);
            }
        }

        let mut err: Id = std::ptr::null_mut();
        let pso = msg_send!(
            raw_dev,
            c"newRenderPipelineStateWithDescriptor:error:",
            desc => Id,
            &mut err as *mut Id => *mut Id;
            -> Id
        );
        msg_send!(desc, c"release");
        msg_send!(vs_fn, c"release");
        msg_send!(fs_fn, c"release");

        if pso.is_null() {
            Err(ns_error_string(err))
        } else {
            Ok(ObjPtr(pso))
        }
    }
}

pub fn shared_pipelines(device: &Device, format: TextureFormat) -> RenderPipelines {
    let mut guard = SHARED_PIPELINES.lock();
    if let Some((fmt, ref pipes)) = *guard
        && fmt == format
    {
        return pipes.clone();
    }
    let pipes = unsafe {
        let raw_dev = device.raw.as_ptr();
        let lib = device.library.as_ptr();
        let cells = build_pipeline(raw_dev, lib, "vs_main", "fs_main", format, BlendMode::Alpha)
            .expect("cells pipeline");
        let corner = build_pipeline(raw_dev, lib, "vs_corner", "fs_corner", format, BlendMode::CornerFade)
            .expect("corner pipeline");
        let image = build_pipeline(raw_dev, lib, "vs_image", "fs_image", format, BlendMode::Alpha)
            .expect("image pipeline");
        RenderPipelines { cells, corner, image }
    };
    *guard = Some((format, pipes.clone()));
    pipes
}

impl Device {
    /// Compile a custom Metal fragment shader library & pipeline (`PostFx`).
    pub fn create_postfx_pipeline(&self, msl_src: &str, format: TextureFormat) -> Result<ObjPtr, String> {
        unsafe {
            let src_ns = ns_string(msl_src).ok_or("invalid MSL source")?;
            let mut err: Id = std::ptr::null_mut();
            let lib = msg_send!(
                self.raw.as_ptr(),
                c"newLibraryWithSource:options:error:",
                src_ns => Id,
                std::ptr::null_mut() => Id,
                &mut err as *mut Id => *mut Id;
                -> Id
            );
            if lib.is_null() {
                return Err(ns_error_string(err));
            }
            let res = build_pipeline(
                self.raw.as_ptr(),
                lib,
                "vs_postfx",
                "fs_postfx",
                format,
                BlendMode::None,
            );
            msg_send!(lib, c"release");
            res
        }
    }

    #[inline]
    pub fn nearest_sampler(&self) -> Id {
        self.nearest_sampler.as_ptr()
    }

    #[inline]
    pub fn linear_sampler(&self) -> Id {
        self.linear_sampler.as_ptr()
    }
}
