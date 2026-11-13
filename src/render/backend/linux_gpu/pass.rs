//! Vulkan command buffer (`CommandEncoder`) and render pass (`RenderPass`) draw encoding.

use std::ffi::c_void;
use std::sync::Arc;

use super::device::{Queue, VulkanCore};
use super::ffi::*;
use super::resources::{HostBuffer, Texture};
use super::surface::SurfaceFrame;
use super::upload::SubmitInfo;

/// Set once a `custom-shader` pass was skipped for want of a descriptor set
/// (reported once: it repeats every frame).
pub(super) static POSTFX_SKIPPED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub struct CommandEncoder {
    vk: Option<Arc<VulkanCore>>,
    /// Whether the frame's command buffer began recording. When it did not,
    /// nothing is recorded and the frame is given up (`give_up_frame`).
    recording: bool,
}

impl CommandEncoder {
    pub fn new(queue: &Queue) -> Self {
        let mut recording = false;
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
                vk.wait_for_last_frame();
                // Every submitted frame has run, and with it every upload batch.
                vk.uploads.lock().free_retired(vk);
                (vk.lib.vkResetCommandBuffer)(vk.upload_cmd, 0);
                recording = vk_try("vkBeginCommandBuffer", || (vk.lib.vkBeginCommandBuffer)(vk.upload_cmd, &b_info as *const _ as *const c_void));
            }
        }
        Self { vk: queue.vk.clone(), recording }
    }

    /// Begin drawing into `target_fb`, a swapchain framebuffer
    /// (`SurfaceFrame::texture_ptr`) or a render target (`Texture::raw_ptr`).
    /// Any other handle records nothing: the returned pass draws nothing.
    pub fn begin_render_pass(&mut self, target_fb: RawTexHandle, clear_rgba: Option<[f64; 4]>) -> RenderPass {
        let target = super::surface::framebuffer_target(target_fb);
        let vk = match (&self.vk, target) {
            (Some(vk), Some(_)) if self.recording => Some(vk.clone()),
            _ => None,
        };
        if let (Some(vk), Some(target)) = (&vk, target) {
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
                let (width, height) = (target.width.max(1), target.height.max(1));
                let [r, g, b, a] = clear_rgba.unwrap_or([0.0, 0.0, 0.0, 1.0]);
                let cv = VkClearColorValue {
                    float32: [r as f32, g as f32, b as f32, a as f32],
                };
                let rp = if clear_rgba.is_some() { target.clear_pass } else { target.load_pass };
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
        // `vk` is `None` unless a render pass was begun, so draws and `Drop`
        // never record into a pass that does not exist.
        RenderPass { vk, call_idx: 0 }
    }

    pub fn present_and_submit(self, frame: SurfaceFrame) {
        let Some(vk) = &self.vk else { return };
        unsafe {
            // Pending uploads run first, in the same submission.
            let mut uploads = vk.uploads.lock();
            let batch = uploads.take_for_frame(vk);
            let mut cmds = [std::ptr::null_mut(); 2];
            let count = match batch {
                Some(batch) => {
                    cmds = [batch, vk.upload_cmd];
                    2
                }
                None => {
                    cmds[0] = vk.upload_cmd;
                    1
                }
            };
            let wait_stage = VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT;
            let submit = SubmitInfo::frame(&cmds[..count], &frame.image_avail_sem, &wait_stage, &frame.render_done_sem);
            let submitted = self.recording
                && vk_try("vkEndCommandBuffer", || (vk.lib.vkEndCommandBuffer)(vk.upload_cmd))
                && vk_try("vkQueueSubmit", || (vk.lib.vkQueueSubmit)(vk.queue, 1, &submit as *const _ as *const c_void, frame.fence));
            if !submitted {
                give_up_frame(vk, batch, frame.image_avail_sem, frame.fence);
                return;
            }
            vk.in_flight.lock().submitted(frame.fence);
            // Presenting can block on the display; uploads need not wait for it.
            drop(uploads);

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
            // Out of date or suboptimal: the next acquire fails or the
            // renderer reconfigures, so only unexpected errors are reported.
            present_ok(reported("vkQueuePresentKHR", || {
                (vk.lib.vkQueuePresentKHR)(vk.queue, &present as *const _ as *const c_void)
            }));
        }
    }

    /// Submit the recorded passes and wait until the GPU has run them.
    #[cfg(test)]
    pub fn submit_and_wait(self) {
        let Some(vk) = &self.vk else { return };
        unsafe {
            let mut uploads = vk.uploads.lock();
            let batch = uploads.take_for_frame(vk);
            let cmds: Vec<VkCommandBuffer> = batch.into_iter().chain([vk.upload_cmd]).collect();
            assert!(vk_try("vkEndCommandBuffer", || (vk.lib.vkEndCommandBuffer)(vk.upload_cmd)));
            let submit = SubmitInfo::commands(&cmds);
            assert!(vk_try("vkQueueSubmit", || (vk.lib.vkQueueSubmit)(vk.queue, 1, &submit as *const _ as *const c_void, 0)));
            (vk.lib.vkQueueWaitIdle)(vk.queue);
            uploads.free_retired(vk);
        }
    }
}

/// Settle a frame whose commands could not be submitted. It is not presented
/// (its image was never drawn), but what the next frames rely on still runs:
/// the upload `batch`, so textures get their contents and layouts; a wait on
/// `image_avail_sem`, which the acquire signaled and must be consumed before
/// it is acquired with again (`VUID-vkAcquireNextImageKHR-semaphore-01286`);
/// and `fence`, which the surface waits for before its next frame.
///
/// # Safety
/// `batch` has ended recording; the semaphore and fence belong to `vk.device`.
pub(super) unsafe fn give_up_frame(vk: &VulkanCore, batch: Option<VkCommandBuffer>, image_avail_sem: VkSemaphore, fence: VkFence) {
    let cmds: &[VkCommandBuffer] = match &batch {
        Some(batch) => std::slice::from_ref(batch),
        None => &[],
    };
    let wait_stage = VK_PIPELINE_STAGE_TRANSFER_BIT;
    let submit = SubmitInfo::waiting(cmds, &image_avail_sem, &wait_stage);
    unsafe {
        if !vk_try("vkQueueSubmit", || (vk.lib.vkQueueSubmit)(vk.queue, 1, &submit as *const _ as *const c_void, fence)) {
            // Still signal the fence so the next frame's wait cannot hang.
            (vk.lib.vkQueueSubmit)(vk.queue, 0, std::ptr::null(), fence);
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
        // A 0 handle is an object that could not be created; binding it is invalid.
        let handles = [pipeline.0, instances.raw_ptr(), viewports.raw_ptr(), atlas.raw_ptr(), color_atlas.raw_ptr(), sampler];
        if instance_count == 0 || handles.contains(&0) {
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
        if pipeline.0 == 0 || dset == 0 || quads.raw_ptr() == 0 {
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

    /// One `custom-shader` pass: the full target, shaded from `source_tex`
    /// (a render target, through its descriptor set and linear sampler) and
    /// `globals` (push constants, so each pass of a frame has its own).
    pub fn draw_postfx<G: bytemuck::Pod>(
        &mut self,
        pipeline: &ObjPtr,
        globals: &G,
        source_tex: &Texture,
        _sampler: SamplerHandle,
    ) {
        const { assert!(std::mem::size_of::<G>() <= super::device::POSTFX_PUSH_SIZE as usize) };
        let Some(vk) = &self.vk else { return };
        let dset = source_tex.image_dset();
        if pipeline.0 == 0 {
            return;
        }
        if dset == 0 {
            // The pass cleared its target, so the window shows black.
            if !POSTFX_SKIPPED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                eprintln!("litty: custom-shader skipped: no descriptor set left for its source (too many images?)");
            }
            return;
        }
        let size = std::mem::size_of::<G>();
        unsafe {
            (vk.lib.vkCmdBindPipeline)(vk.upload_cmd, 0, pipeline.0);
            (vk.lib.vkCmdBindDescriptorSets)(vk.upload_cmd, 0, vk.postfx_pipe_layout, 0, 1, &dset, 0, std::ptr::null());
            (vk.lib.vkCmdPushConstants)(
                vk.upload_cmd,
                vk.postfx_pipe_layout,
                VK_SHADER_STAGE_FRAGMENT_BIT,
                0,
                size as u32,
                globals as *const G as *const c_void,
            );
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

#[cfg(test)]
mod tests {
    use super::*;

    /// An image view passed as a render target used to be handed to
    /// `vkCmdBeginRenderPass` as a framebuffer and crashed the driver.
    #[test]
    fn an_unknown_target_records_no_render_pass() {
        let _gpu = fault::GPU.lock();
        let (_device, queue) = super::super::device::shared_device_and_queue();
        let mut encoder = CommandEncoder::new(&queue);
        let pass = encoder.begin_render_pass(0xDEAD_0002, Some([0.0; 4]));
        assert!(pass.vk.is_none(), "no pass begun, so draws and Drop record nothing");
    }

    /// When `vkBeginCommandBuffer` failed, a render pass recorded into the
    /// command buffer anyway (it is not recording: undefined behavior).
    #[test]
    fn a_frame_that_did_not_begin_records_no_render_pass() {
        let _gpu = fault::GPU.lock();
        let (_device, queue) = super::super::device::shared_device_and_queue();
        let fb = 0xDEAD_0003;
        let target = super::super::surface::FramebufferTarget { width: 4, height: 4, clear_pass: 7, load_pass: 8 };
        super::super::surface::register_framebuffer(fb, target);
        // As `CommandEncoder::new` leaves it after a failed begin.
        let mut encoder = CommandEncoder { vk: queue.vk.clone(), recording: false };
        let pass = encoder.begin_render_pass(fb, Some([0.0; 4]));
        super::super::surface::forget_framebuffer(fb);
        assert!(pass.vk.is_none(), "no pass begun on a command buffer that is not recording");
    }
}
