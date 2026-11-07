//! Metal command buffer (`CommandEncoder`) and render pass (`RenderPass`) draw encoding.

use std::ffi::c_void;

use super::device::Queue;
use super::ffi::*;
use super::resources::{HostBuffer, Texture};
use super::surface::SurfaceFrame;

/// Command buffer wrapper (`MTLCommandBuffer`).
pub struct CommandEncoder {
    cmd_buf: ObjPtr,
}

impl CommandEncoder {
    pub fn new(queue: &Queue) -> Self {
        unsafe {
            let raw = msg_send!(queue.raw.as_ptr(), c"commandBuffer"; -> Id);
            assert!(!raw.is_null(), "MTLCommandQueue commandBuffer returned null");
            msg_send!(raw, c"retain"; -> Id);
            Self { cmd_buf: ObjPtr(raw) }
        }
    }

    pub fn begin_render_pass(&mut self, target_tex: Id, clear_rgba: Option<[f64; 4]>) -> RenderPass {
        unsafe {
            let cls = class(c"MTLRenderPassDescriptor").expect("MTLRenderPassDescriptor");
            let desc = msg_send!(cls, c"renderPassDescriptor"; -> Id);
            let attachments = msg_send!(desc, c"colorAttachments"; -> Id);
            let att0 = msg_send!(attachments, c"objectAtIndexedSubscript:", 0usize => usize; -> Id);
            msg_send!(att0, c"setTexture:", target_tex => Id);
            if let Some([r, g, b, a]) = clear_rgba {
                msg_send!(att0, c"setLoadAction:", MTL_LOAD_ACTION_CLEAR => usize);
                msg_send!(
                    att0,
                    c"setClearColor:",
                    MTLClearColor { red: r, green: g, blue: b, alpha: a } => MTLClearColor
                );
            } else {
                msg_send!(att0, c"setLoadAction:", MTL_LOAD_ACTION_LOAD => usize);
            }
            msg_send!(att0, c"setStoreAction:", MTL_STORE_ACTION_STORE => usize);

            let enc = msg_send!(
                self.cmd_buf.as_ptr(),
                c"renderCommandEncoderWithDescriptor:",
                desc => Id;
                -> Id
            );
            assert!(!enc.is_null(), "renderCommandEncoderWithDescriptor returned null");
            msg_send!(enc, c"retain"; -> Id);
            RenderPass { encoder: ObjPtr(enc) }
        }
    }

    pub fn present_and_submit(self, frame: SurfaceFrame) {
        unsafe {
            msg_send!(self.cmd_buf.as_ptr(), c"presentDrawable:", frame.drawable.as_ptr() => Id);
            msg_send!(self.cmd_buf.as_ptr(), c"commit");
        }
    }

    pub fn present_and_submit_wait(self, frame: SurfaceFrame) {
        unsafe {
            msg_send!(self.cmd_buf.as_ptr(), c"presentDrawable:", frame.drawable.as_ptr() => Id);
            msg_send!(self.cmd_buf.as_ptr(), c"commit");
            msg_send!(self.cmd_buf.as_ptr(), c"waitUntilCompleted");
        }
    }

    #[cfg(test)]
    pub fn submit_and_wait(self) {
        unsafe {
            msg_send!(self.cmd_buf.as_ptr(), c"commit");
            msg_send!(self.cmd_buf.as_ptr(), c"waitUntilCompleted");
        }
    }
}

/// Active render pass encoder (`MTLRenderCommandEncoder`).
pub struct RenderPass {
    encoder: ObjPtr,
}

impl RenderPass {
    #[inline]
    pub fn set_scissor_rect(&mut self, x: u32, y: u32, width: u32, height: u32) {
        let rect = MTLScissorRect {
            x: x as usize,
            y: y as usize,
            width: width as usize,
            height: height as usize,
        };
        unsafe {
            msg_send!(self.encoder.as_ptr(), c"setScissorRect:", rect => MTLScissorRect);
        }
    }

    /// Bind and draw instanced terminal cells (`0..4` triangle strip, `0..instance_count`).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_cells<I: bytemuck::Pod, U: bytemuck::Pod, V: bytemuck::Pod>(
        &mut self,
        pipeline: &ObjPtr,
        instances: &HostBuffer<I>,
        uniforms: &U,
        viewports: &HostBuffer<V>,
        atlas: &Texture,
        color_atlas: &Texture,
        sampler: Id,
        instance_count: u32,
    ) {
        if instance_count == 0 {
            return;
        }
        unsafe {
            let enc = self.encoder.as_ptr();
            msg_send!(enc, c"setRenderPipelineState:", pipeline.as_ptr() => Id);
            msg_send!(enc, c"setVertexBuffer:offset:atIndex:", instances.raw_ptr() => Id, 0usize => usize, 0usize => usize);
            msg_send!(
                enc,
                c"setVertexBytes:length:atIndex:",
                uniforms as *const U as *const c_void => *const c_void,
                std::mem::size_of::<U>() => usize,
                1usize => usize
            );
            msg_send!(enc, c"setVertexBuffer:offset:atIndex:", viewports.raw_ptr() => Id, 0usize => usize, 2usize => usize);
            msg_send!(
                enc,
                c"setFragmentBytes:length:atIndex:",
                uniforms as *const U as *const c_void => *const c_void,
                std::mem::size_of::<U>() => usize,
                1usize => usize
            );
            msg_send!(enc, c"setFragmentTexture:atIndex:", atlas.raw_ptr() => Id, 0usize => usize);
            msg_send!(enc, c"setFragmentTexture:atIndex:", color_atlas.raw_ptr() => Id, 1usize => usize);
            msg_send!(enc, c"setFragmentSamplerState:atIndex:", sampler => Id, 0usize => usize);
            msg_send!(
                enc,
                c"drawPrimitives:vertexStart:vertexCount:instanceCount:",
                MTL_PRIMITIVE_TYPE_TRIANGLE_STRIP => usize,
                0usize => usize,
                4usize => usize,
                instance_count as usize => usize
            );
        }
    }

    /// Bind and draw a single inline image quad instance `instance_idx`.
    pub fn draw_image_quad<Q: bytemuck::Pod, U: bytemuck::Pod>(
        &mut self,
        pipeline: &ObjPtr,
        quads: &HostBuffer<Q>,
        uniforms: &U,
        texture: &Texture,
        sampler: Id,
        instance_idx: u32,
    ) {
        unsafe {
            let enc = self.encoder.as_ptr();
            msg_send!(enc, c"setRenderPipelineState:", pipeline.as_ptr() => Id);
            msg_send!(enc, c"setVertexBuffer:offset:atIndex:", quads.raw_ptr() => Id, 0usize => usize, 0usize => usize);
            msg_send!(
                enc,
                c"setVertexBytes:length:atIndex:",
                uniforms as *const U as *const c_void => *const c_void,
                std::mem::size_of::<U>() => usize,
                1usize => usize
            );
            msg_send!(enc, c"setFragmentTexture:atIndex:", texture.raw_ptr() => Id, 0usize => usize);
            msg_send!(enc, c"setFragmentSamplerState:atIndex:", sampler => Id, 0usize => usize);
            msg_send!(
                enc,
                c"drawPrimitives:vertexStart:vertexCount:instanceCount:baseInstance:",
                MTL_PRIMITIVE_TYPE_TRIANGLE_STRIP => usize,
                0usize => usize,
                4usize => usize,
                1usize => usize,
                instance_idx as usize => usize
            );
        }
    }

    /// Draw the rounded window corner fade pass (`vs_corner` / `fs_corner`).
    pub fn draw_corners<U: bytemuck::Pod>(&mut self, pipeline: &ObjPtr, uniforms: &U) {
        unsafe {
            let enc = self.encoder.as_ptr();
            msg_send!(enc, c"setRenderPipelineState:", pipeline.as_ptr() => Id);
            msg_send!(
                enc,
                c"setFragmentBytes:length:atIndex:",
                uniforms as *const U as *const c_void => *const c_void,
                std::mem::size_of::<U>() => usize,
                1usize => usize
            );
            msg_send!(
                enc,
                c"drawPrimitives:vertexStart:vertexCount:instanceCount:",
                MTL_PRIMITIVE_TYPE_TRIANGLE_STRIP => usize,
                0usize => usize,
                4usize => usize,
                1usize => usize
            );
        }
    }

    /// Draw a full-screen post-processing triangle (`0..3`).
    pub fn draw_postfx<G: bytemuck::Pod>(
        &mut self,
        pipeline: &ObjPtr,
        globals: &G,
        source_tex: &Texture,
        sampler: Id,
    ) {
        unsafe {
            let enc = self.encoder.as_ptr();
            msg_send!(enc, c"setRenderPipelineState:", pipeline.as_ptr() => Id);
            msg_send!(
                enc,
                c"setFragmentBytes:length:atIndex:",
                globals as *const G as *const c_void => *const c_void,
                std::mem::size_of::<G>() => usize,
                0usize => usize
            );
            msg_send!(enc, c"setFragmentTexture:atIndex:", source_tex.raw_ptr() => Id, 0usize => usize);
            msg_send!(enc, c"setFragmentSamplerState:atIndex:", sampler => Id, 0usize => usize);
            msg_send!(
                enc,
                c"drawPrimitives:vertexStart:vertexCount:instanceCount:",
                MTL_PRIMITIVE_TYPE_TRIANGLE => usize,
                0usize => usize,
                3usize => usize,
                1usize => usize
            );
        }
    }
}

impl Drop for RenderPass {
    fn drop(&mut self) {
        unsafe {
            msg_send!(self.encoder.as_ptr(), c"endEncoding");
        }
    }
}
