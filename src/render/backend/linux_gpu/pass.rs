//! Vulkan command buffer (`CommandEncoder`) and render pass (`RenderPass`) draw encoding.

use std::ffi::c_void;
use std::sync::Arc;

use super::device::{Queue, VulkanCore, SHARED_PIPES};
use super::ffi::*;
use super::resources::{HostBuffer, Texture};
use super::surface::SurfaceFrame;

pub struct CommandEncoder {
    vk: Option<Arc<VulkanCore>>,
    width: u32,
    height: u32,
}

impl CommandEncoder {
    pub fn new(queue: &Queue) -> Self {
        if let Some(vk) = &queue.vk {
            unsafe {
                #[repr(C)]
                struct BeginInfo {
                    s_type: u32,
                    p_next: *const c_void,
                    flags: u32,
                    p_inheritance_info: *const c_void,
                }
                let b_info = BeginInfo {
                    s_type: VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
                    p_next: std::ptr::null(),
                    flags: 1,
                    p_inheritance_info: std::ptr::null(),
                };
                (vk.lib.vkResetCommandBuffer)(vk.upload_cmd, 0);
                (vk.lib.vkBeginCommandBuffer)(vk.upload_cmd, &b_info as *const _ as *const c_void);
            }
        }
        Self {
            vk: queue.vk.clone(),
            width: 1,
            height: 1,
        }
    }

    pub fn begin_render_pass(&mut self, target_fb: RawTexHandle, clear_rgba: Option<[f64; 4]>) -> RenderPass {
        if let Some(vk) = &self.vk
            && target_fb != 0
        {
            let guard = SHARED_PIPES.lock();
            if let Some((_, ref pipes)) = *guard {
                unsafe {
                    #[repr(C)]
                    struct RpBegin {
                        s_type: u32,
                        p_next: *const c_void,
                        render_pass: VkRenderPass,
                        framebuffer: VkFramebuffer,
                        render_area: VkRect2D,
                        clear_value_count: u32,
                        p_clear_values: *const VkClearColorValue,
                    }
                    if let Some((w, h)) = super::surface::framebuffer_extent(target_fb) {
                        self.width = w;
                        self.height = h;
                    }
                    let (width, height) = (self.width.max(1), self.height.max(1));
                    let [r, g, b, a] = clear_rgba.unwrap_or([0.0, 0.0, 0.0, 1.0]);
                    let cv = VkClearColorValue {
                        float32: [r as f32, g as f32, b as f32, a as f32],
                    };
                    let rp = if clear_rgba.is_some() { pipes.render_pass_clear } else { pipes.render_pass_load };
                    let begin = RpBegin {
                        s_type: VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO,
                        p_next: std::ptr::null(),
                        render_pass: rp,
                        framebuffer: target_fb,
                        render_area: VkRect2D {
                            offset: VkOffset2D { x: 0, y: 0 },
                            extent: VkExtent2D { width, height },
                        },
                        clear_value_count: 1,
                        p_clear_values: &cv,
                    };
                    (vk.lib.vkCmdBeginRenderPass)(vk.upload_cmd, &begin as *const _ as *const c_void, 0);
                    // Viewport and scissor are dynamic state: without them
                    // nothing outside the render area's first pixel is drawn.
                    let viewport = VkViewport {
                        x: 0.0,
                        y: 0.0,
                        width: width as f32,
                        height: height as f32,
                        min_depth: 0.0,
                        max_depth: 1.0,
                    };
                    (vk.lib.vkCmdSetViewport)(vk.upload_cmd, 0, 1, &viewport);
                    (vk.lib.vkCmdSetScissor)(vk.upload_cmd, 0, 1, &begin.render_area);
                }
            }
        }
        RenderPass {
            vk: self.vk.clone(),
            call_idx: 0,
        }
    }

    pub fn present_and_submit(self, frame: SurfaceFrame) {
        let Some(vk) = &self.vk else { return };
        unsafe {
            (vk.lib.vkEndCommandBuffer)(vk.upload_cmd);
            #[repr(C)]
            struct SubmitInfo {
                s_type: u32,
                p_next: *const c_void,
                wait_semaphore_count: u32,
                p_wait_semaphores: *const VkSemaphore,
                p_wait_dst_stage_mask: *const VkFlags,
                command_buffer_count: u32,
                p_command_buffers: *const VkCommandBuffer,
                signal_semaphore_count: u32,
                p_signal_semaphores: *const VkSemaphore,
            }
            let wait_stage = VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT;
            let submit = SubmitInfo {
                s_type: VK_STRUCTURE_TYPE_SUBMIT_INFO,
                p_next: std::ptr::null(),
                wait_semaphore_count: 1,
                p_wait_semaphores: &frame.image_avail_sem,
                p_wait_dst_stage_mask: &wait_stage,
                command_buffer_count: 1,
                p_command_buffers: &vk.upload_cmd,
                signal_semaphore_count: 1,
                p_signal_semaphores: &frame.render_done_sem,
            };
            (vk.lib.vkQueueSubmit)(vk.queue, 1, &submit as *const _ as *const c_void, frame.fence);

            #[repr(C)]
            struct PresentInfo {
                s_type: u32,
                p_next: *const c_void,
                wait_semaphore_count: u32,
                p_wait_semaphores: *const VkSemaphore,
                swapchain_count: u32,
                p_swapchains: *const VkSwapchainKHR,
                p_image_indices: *const u32,
                p_results: *mut i32,
            }
            let present = PresentInfo {
                s_type: VK_STRUCTURE_TYPE_PRESENT_INFO_KHR,
                p_next: std::ptr::null(),
                wait_semaphore_count: 1,
                p_wait_semaphores: &frame.render_done_sem,
                swapchain_count: 1,
                p_swapchains: &frame.swapchain,
                p_image_indices: &frame.image_index,
                p_results: std::ptr::null_mut(),
            };
            (vk.lib.vkQueuePresentKHR)(vk.queue, &present as *const _ as *const c_void);
        }
    }

    #[cfg(test)]
    pub fn submit_and_wait(self) {
        let Some(vk) = &self.vk else { return };
        unsafe {
            (vk.lib.vkEndCommandBuffer)(vk.upload_cmd);
            (vk.lib.vkQueueWaitIdle)(vk.queue);
        }
    }
}

pub struct RenderPass {
    vk: Option<Arc<VulkanCore>>,
    call_idx: usize,
}

impl RenderPass {
    #[inline]
    pub fn set_scissor_rect(&mut self, x: u32, y: u32, width: u32, height: u32) {
        if let Some(vk) = &self.vk {
            let rect = VkRect2D {
                offset: VkOffset2D { x: x as i32, y: y as i32 },
                extent: VkExtent2D { width, height },
            };
            unsafe {
                (vk.lib.vkCmdSetScissor)(vk.upload_cmd, 0, 1, &rect);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_cells<I: bytemuck::Pod, U: bytemuck::Pod, V: bytemuck::Pod>(
        &mut self,
        pipeline: &ObjPtr,
        instances: &HostBuffer<I>,
        uniforms: &U,
        viewports: &HostBuffer<V>,
        atlas: &Texture,
        color_atlas: &Texture,
        sampler: SamplerHandle,
        instance_count: u32,
    ) {
        let Some(vk) = &self.vk else { return };
        if instance_count == 0 || pipeline.0 == 0 {
            return;
        }
        let slot = self.call_idx & 1;
        self.call_idx += 1;
        unsafe {
            let (ubuf, uptr) = vk.cells_ubufs[slot];
            std::ptr::copy_nonoverlapping(
                uniforms as *const U as *const u8,
                uptr,
                std::mem::size_of::<U>(),
            );

            #[repr(C)]
            struct BufDesc {
                buffer: VkBuffer,
                offset: VkDeviceSize,
                range: VkDeviceSize,
            }
            #[repr(C)]
            struct ImgDesc {
                sampler: VkSampler,
                image_view: VkImageView,
                image_layout: u32,
            }
            #[repr(C)]
            #[derive(Copy, Clone)]
            struct WriteDesc {
                s_type: u32,
                p_next: *const c_void,
                dst_set: VkDescriptorSet,
                dst_binding: u32,
                dst_array_element: u32,
                descriptor_count: u32,
                descriptor_type: u32,
                p_image_info: *const ImgDesc,
                p_buffer_info: *const BufDesc,
                p_texel_buffer_view: *const c_void,
            }

            let dset = vk.cells_dsets[slot];
            let b0 = BufDesc { buffer: ubuf, offset: 0, range: std::mem::size_of::<U>() as u64 };
            let b4 = BufDesc { buffer: viewports.raw_ptr(), offset: 0, range: u64::MAX };
            let i1 = ImgDesc { sampler: 0, image_view: atlas.raw_ptr(), image_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL };
            let i2 = ImgDesc { sampler, image_view: 0, image_layout: 0 };
            let i3 = ImgDesc { sampler: 0, image_view: color_atlas.raw_ptr(), image_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL };

            let writes = [
                WriteDesc { s_type: VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET, p_next: std::ptr::null(), dst_set: dset, dst_binding: 0, dst_array_element: 0, descriptor_count: 1, descriptor_type: VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER, p_image_info: std::ptr::null(), p_buffer_info: &b0, p_texel_buffer_view: std::ptr::null() },
                WriteDesc { s_type: VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET, p_next: std::ptr::null(), dst_set: dset, dst_binding: 1, dst_array_element: 0, descriptor_count: 1, descriptor_type: VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE, p_image_info: &i1, p_buffer_info: std::ptr::null(), p_texel_buffer_view: std::ptr::null() },
                WriteDesc { s_type: VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET, p_next: std::ptr::null(), dst_set: dset, dst_binding: 2, dst_array_element: 0, descriptor_count: 1, descriptor_type: VK_DESCRIPTOR_TYPE_SAMPLER, p_image_info: &i2, p_buffer_info: std::ptr::null(), p_texel_buffer_view: std::ptr::null() },
                WriteDesc { s_type: VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET, p_next: std::ptr::null(), dst_set: dset, dst_binding: 3, dst_array_element: 0, descriptor_count: 1, descriptor_type: VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE, p_image_info: &i3, p_buffer_info: std::ptr::null(), p_texel_buffer_view: std::ptr::null() },
                WriteDesc { s_type: VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET, p_next: std::ptr::null(), dst_set: dset, dst_binding: 4, dst_array_element: 0, descriptor_count: 1, descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER, p_image_info: std::ptr::null(), p_buffer_info: &b4, p_texel_buffer_view: std::ptr::null() },
            ];
            (vk.lib.vkUpdateDescriptorSets)(vk.device, writes.len() as u32, writes.as_ptr() as *const c_void, 0, std::ptr::null());

            (vk.lib.vkCmdBindPipeline)(vk.upload_cmd, 0, pipeline.0);
            (vk.lib.vkCmdBindDescriptorSets)(vk.upload_cmd, 0, vk.cells_pipe_layout, 0, 1, &dset, 0, std::ptr::null());
            let vbuf = instances.raw_ptr();
            let offset = 0u64;
            (vk.lib.vkCmdBindVertexBuffers)(vk.upload_cmd, 0, 1, &vbuf, &offset);
            (vk.lib.vkCmdDraw)(vk.upload_cmd, 4, instance_count, 0, 0);
        }
    }

    pub fn draw_image_quad<Q: bytemuck::Pod, U: bytemuck::Pod>(
        &mut self,
        pipeline: &ObjPtr,
        quads: &HostBuffer<Q>,
        uniforms: &U,
        texture: &Texture,
        _sampler: SamplerHandle,
        instance_idx: u32,
    ) {
        let Some(vk) = &self.vk else { return };
        let dset = texture.image_dset();
        if pipeline.0 == 0 || dset == 0 {
            return;
        }
        unsafe {
            // Every image of a frame shares these uniforms (target size, flip).
            std::ptr::copy_nonoverlapping(
                uniforms as *const U as *const u8,
                vk.image_ubuf.1,
                std::mem::size_of::<U>(),
            );
            (vk.lib.vkCmdBindPipeline)(vk.upload_cmd, 0, pipeline.0);
            (vk.lib.vkCmdBindDescriptorSets)(vk.upload_cmd, 0, vk.image_pipe_layout, 0, 1, &dset, 0, std::ptr::null());
            let vbuf = quads.raw_ptr();
            let offset = 0u64;
            (vk.lib.vkCmdBindVertexBuffers)(vk.upload_cmd, 0, 1, &vbuf, &offset);
            (vk.lib.vkCmdDraw)(vk.upload_cmd, 4, 1, 0, instance_idx);
        }
    }

    pub fn draw_corners<U: bytemuck::Pod>(&mut self, pipeline: &ObjPtr, _uniforms: &U) {
        let Some(vk) = &self.vk else { return };
        if pipeline.0 == 0 {
            return;
        }
        unsafe {
            (vk.lib.vkCmdBindPipeline)(vk.upload_cmd, 0, pipeline.0);
            // An image draw leaves an image-layout set bound; the corner shader
            // reads the cell uniforms the frame's first `draw_cells` wrote.
            let dset = vk.cells_dsets[0];
            (vk.lib.vkCmdBindDescriptorSets)(vk.upload_cmd, 0, vk.cells_pipe_layout, 0, 1, &dset, 0, std::ptr::null());
            (vk.lib.vkCmdDraw)(vk.upload_cmd, 4, 1, 0, 0);
        }
    }

    pub fn draw_postfx<G: bytemuck::Pod>(
        &mut self,
        pipeline: &ObjPtr,
        _globals: &G,
        _source_tex: &Texture,
        _sampler: SamplerHandle,
    ) {
        let Some(vk) = &self.vk else { return };
        if pipeline.0 == 0 {
            return;
        }
        unsafe {
            (vk.lib.vkCmdBindPipeline)(vk.upload_cmd, 0, pipeline.0);
            (vk.lib.vkCmdDraw)(vk.upload_cmd, 3, 1, 0, 0);
        }
    }
}

impl Drop for RenderPass {
    fn drop(&mut self) {
        if let Some(vk) = &self.vk {
            unsafe {
                (vk.lib.vkCmdEndRenderPass)(vk.upload_cmd);
            }
        }
    }
}
