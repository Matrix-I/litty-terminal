//! Texture uploads batched into one command buffer, submitted with the frame.
//!
//! `Texture::write_rect` copies its pixels into a staging buffer and records
//! the copy into the batch. The next frame submits the batch ahead of its own
//! command buffer in the same `vkQueueSubmit`, so a frame with many new
//! glyphs costs no extra submission or wait. Work that needs the uploads to
//! have run without a frame (destroying a texture, reading pixels back, a
//! full staging buffer) flushes the batch on its own and waits for it.

use std::ffi::c_void;

use super::device::VulkanCore;
use super::ffi::*;
use super::resources::alloc_host_buffer;

/// Smallest shared staging buffer: a few hundred glyph cells.
const STAGING_MIN: usize = 64 << 10;
/// Largest staging buffer kept between batches; bigger uploads (inline
/// images) get a buffer of their own so idle memory does not hold their size.
const STAGING_KEEP_MAX: usize = 4 << 20;
/// Offset alignment of each upload in the staging buffer: a multiple of
/// every texel size, which `vkCmdCopyBufferToImage` requires.
const STAGING_ALIGN: usize = 16;

/// Where a `len`-byte upload goes when `used` of the `cap` staging bytes
/// already hold this batch's uploads.
#[derive(Debug, PartialEq, Eq)]
enum Placement {
    /// At this offset of the shared staging buffer.
    Shared(usize),
    /// The batch's uploads fill the buffer: flush them, make the buffer at
    /// least this large, and put the upload at offset 0.
    FlushAndGrow(usize),
    /// Too large to keep: a buffer of its own, freed once the batch has run.
    OneOff,
}

fn place(used: usize, cap: usize, len: usize) -> Placement {
    if len > STAGING_KEEP_MAX {
        return Placement::OneOff;
    }
    let offset = used.next_multiple_of(STAGING_ALIGN);
    if offset + len <= cap {
        return Placement::Shared(offset);
    }
    // Room for a batch as large as this one, so the next does not flush.
    Placement::FlushAndGrow((offset + len).next_power_of_two().clamp(STAGING_MIN, STAGING_KEEP_MAX).max(cap))
}

/// Upload staging memory shared by every texture of the device.
struct Staging {
    buf: VkBuffer,
    mem: VkDeviceMemory,
    ptr: *mut u8,
    cap: usize,
}

/// The upload batch of the device. All of it is behind `VulkanCore::uploads`.
pub(super) struct Uploads {
    cmd: VkCommandBuffer,
    recording: bool,
    staging: Staging,
    /// Staging bytes holding this batch's uploads.
    used: usize,
    /// Buffers of this batch's oversized uploads.
    one_offs: Vec<(VkBuffer, VkDeviceMemory)>,
    /// Buffers of a batch submitted with a frame, freed once frames finished.
    retired: Vec<(VkBuffer, VkDeviceMemory)>,
}

impl Uploads {
    pub(super) const fn new(cmd: VkCommandBuffer) -> Self {
        Self {
            cmd,
            recording: false,
            staging: Staging { buf: 0, mem: 0, ptr: std::ptr::null_mut(), cap: 0 },
            used: 0,
            one_offs: Vec::new(),
            retired: Vec::new(),
        }
    }

    /// The batch's command buffer, recording. Starting a batch waits for
    /// the last frame, which may still run the previous batch and read the
    /// staging buffer. `None` when recording could not start.
    pub(super) unsafe fn begin(&mut self, vk: &VulkanCore) -> Option<VkCommandBuffer> {
        if self.recording {
            return Some(self.cmd);
        }
        #[repr(C)]
        struct BeginInfo {
            s_type: u32,
            p_next: *const c_void,
            flags: u32,
            p_inheritance_info: *const c_void,
        }
        let info = BeginInfo {
            s_type: VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
            p_next: std::ptr::null(),
            flags: 1,
            p_inheritance_info: std::ptr::null(),
        };
        vk.wait_for_last_frame();
        unsafe {
            self.free_retired(vk);
            (vk.lib.vkResetCommandBuffer)(self.cmd, 0);
            if !vk_ok((vk.lib.vkBeginCommandBuffer)(self.cmd, &info as *const _ as *const c_void), "vkBeginCommandBuffer") {
                return None;
            }
        }
        self.recording = true;
        self.used = 0;
        Some(self.cmd)
    }

    /// Copy `data` into staging memory for this batch; returns the buffer
    /// and offset to copy from. `None` (reported) when memory ran out.
    pub(super) unsafe fn stage(&mut self, vk: &VulkanCore, data: &[u8]) -> Option<(VkBuffer, VkDeviceSize)> {
        unsafe {
            self.begin(vk)?;
            let offset = match place(self.used, self.staging.cap, data.len()) {
                Placement::Shared(offset) => offset,
                Placement::OneOff => {
                    let (buf, mem, ptr) = alloc_host_buffer(&vk.lib, vk.device, &vk.mem_props, data.len(), VK_BUFFER_USAGE_TRANSFER_SRC_BIT)?;
                    std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
                    self.one_offs.push((buf, mem));
                    return Some((buf, 0));
                }
                Placement::FlushAndGrow(cap) => {
                    if self.used > 0 {
                        // The recorded copies read the buffer: run them first.
                        self.flush(vk);
                        self.begin(vk)?;
                    }
                    if cap > self.staging.cap {
                        // Nothing in flight reads it: `begin` waited for the last frame.
                        self.free_staging(vk);
                        let (buf, mem, ptr) = alloc_host_buffer(&vk.lib, vk.device, &vk.mem_props, cap, VK_BUFFER_USAGE_TRANSFER_SRC_BIT)?;
                        self.staging = Staging { buf, mem, ptr, cap };
                    }
                    0
                }
            };
            std::ptr::copy_nonoverlapping(data.as_ptr(), self.staging.ptr.add(offset), data.len());
            self.used = offset + data.len();
            Some((self.staging.buf, offset as VkDeviceSize))
        }
    }

    /// End the batch for the frame about to be submitted; the caller submits
    /// the returned command buffer before the frame's own. `None` when no
    /// upload is pending.
    pub(super) unsafe fn take_for_frame(&mut self, vk: &VulkanCore) -> Option<VkCommandBuffer> {
        if !self.recording {
            return None;
        }
        self.recording = false;
        self.retired.append(&mut self.one_offs);
        // SAFETY: `cmd` is recording.
        let ended = unsafe { (vk.lib.vkEndCommandBuffer)(self.cmd) };
        vk_ok(ended, "vkEndCommandBuffer").then_some(self.cmd)
    }

    /// Submit the batch on its own and wait until it has run.
    pub(super) unsafe fn flush(&mut self, vk: &VulkanCore) {
        unsafe {
            let Some(cmd) = self.take_for_frame(vk) else { return };
            // Bound to a local: `submit` points into it until the submit returns.
            let cmds = [cmd];
            let submit = SubmitInfo::commands(&cmds);
            // A failed submit uploads nothing; the textures keep their old contents.
            vk_ok((vk.lib.vkQueueSubmit)(vk.queue, 1, &submit as *const _ as *const c_void, 0), "vkQueueSubmit");
            (vk.lib.vkQueueWaitIdle)(vk.queue);
            self.free_retired(vk);
        }
    }

    /// Free the buffers of batches that have run. The caller has waited for
    /// the last submitted frame (or the queue).
    pub(super) unsafe fn free_retired(&mut self, vk: &VulkanCore) {
        for (buf, mem) in self.retired.drain(..) {
            // SAFETY: created from `vk.device`; no submitted work uses them any more.
            unsafe {
                (vk.lib.vkDestroyBuffer)(vk.device, buf, std::ptr::null());
                (vk.lib.vkFreeMemory)(vk.device, mem, std::ptr::null());
            }
        }
    }

    unsafe fn free_staging(&mut self, vk: &VulkanCore) {
        if self.staging.buf != 0 {
            // SAFETY: created from `vk.device`; the caller made sure nothing reads it.
            unsafe {
                (vk.lib.vkDestroyBuffer)(vk.device, self.staging.buf, std::ptr::null());
                (vk.lib.vkFreeMemory)(vk.device, self.staging.mem, std::ptr::null());
            }
        }
        self.staging = Staging { buf: 0, mem: 0, ptr: std::ptr::null_mut(), cap: 0 };
    }
}

/// `VkSubmitInfo`.
#[repr(C)]
pub(super) struct SubmitInfo {
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

impl SubmitInfo {
    /// Run `cmds` in order, without semaphores. `cmds` must outlive the submit.
    pub(super) fn commands(cmds: &[VkCommandBuffer]) -> Self {
        Self {
            s_type: VK_STRUCTURE_TYPE_SUBMIT_INFO,
            p_next: std::ptr::null(),
            wait_semaphore_count: 0,
            p_wait_semaphores: std::ptr::null(),
            p_wait_dst_stage_mask: std::ptr::null(),
            command_buffer_count: cmds.len() as u32,
            p_command_buffers: cmds.as_ptr(),
            signal_semaphore_count: 0,
            p_signal_semaphores: std::ptr::null(),
        }
    }

    /// Wait for `wait` (at `wait_stage`) before running `cmds`, then signal `signal`.
    pub(super) fn frame(cmds: &[VkCommandBuffer], wait: &VkSemaphore, wait_stage: &VkFlags, signal: &VkSemaphore) -> Self {
        Self {
            wait_semaphore_count: 1,
            p_wait_semaphores: wait,
            p_wait_dst_stage_mask: wait_stage,
            signal_semaphore_count: 1,
            p_signal_semaphores: signal,
            ..Self::commands(cmds)
        }
    }
}

/// Record a layout transition of the whole color image into `cmd`:
/// `layouts` = (old, new), `access` = (src, dst), `stages` = (src, dst).
pub(super) unsafe fn image_barrier(
    vk: &VulkanCore,
    cmd: VkCommandBuffer,
    image: VkImage,
    layouts: (u32, u32),
    access: (VkFlags, VkFlags),
    stages: (VkFlags, VkFlags),
) {
    #[repr(C)]
    struct ImgBarrier {
        s_type: u32,
        p_next: *const c_void,
        src_access_mask: VkFlags,
        dst_access_mask: VkFlags,
        old_layout: u32,
        new_layout: u32,
        src_queue_family_index: u32,
        dst_queue_family_index: u32,
        image: VkImage,
        subresource_range: [u32; 5],
    }
    let barrier = ImgBarrier {
        s_type: VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER,
        p_next: std::ptr::null(),
        src_access_mask: access.0,
        dst_access_mask: access.1,
        old_layout: layouts.0,
        new_layout: layouts.1,
        src_queue_family_index: u32::MAX,
        dst_queue_family_index: u32::MAX,
        image,
        subresource_range: [1, 0, 1, 0, 1],
    };
    // SAFETY: `cmd` is recording and `barrier` outlives the call.
    unsafe {
        (vk.lib.vkCmdPipelineBarrier)(
            cmd,
            stages.0,
            stages.1,
            0,
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            1,
            &barrier as *const _ as *const c_void,
        );
    }
}

/// Record a global memory barrier into `cmd`: `access` = (src, dst), `stages` = (src, dst).
#[cfg(test)]
pub(super) unsafe fn memory_barrier(vk: &VulkanCore, cmd: VkCommandBuffer, access: (VkFlags, VkFlags), stages: (VkFlags, VkFlags)) {
    #[repr(C)]
    struct MemBarrier {
        s_type: u32,
        p_next: *const c_void,
        src_access_mask: VkFlags,
        dst_access_mask: VkFlags,
    }
    let barrier = MemBarrier {
        s_type: VK_STRUCTURE_TYPE_MEMORY_BARRIER,
        p_next: std::ptr::null(),
        src_access_mask: access.0,
        dst_access_mask: access.1,
    };
    // SAFETY: `cmd` is recording and `barrier` outlives the call.
    unsafe {
        (vk.lib.vkCmdPipelineBarrier)(
            cmd,
            stages.0,
            stages.1,
            0,
            1,
            &barrier as *const _ as *const c_void,
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uploads_share_the_staging_buffer_until_it_is_full() {
        // A glyph after another lands right behind it, aligned for the copy.
        assert_eq!(place(0, 64 << 10, 512), Placement::Shared(0));
        assert_eq!(place(512, 64 << 10, 512), Placement::Shared(512));
        assert_eq!(place(513, 64 << 10, 512), Placement::Shared(528));
        assert_eq!(place((64 << 10) - 512, 64 << 10, 512), Placement::Shared((64 << 10) - 512));
        // Full: flush and double, so a batch this size fits next time.
        assert_eq!(place((64 << 10) - 256, 64 << 10, 512), Placement::FlushAndGrow(128 << 10));
    }

    #[test]
    fn the_staging_buffer_starts_small_and_stays_bounded() {
        assert_eq!(place(0, 0, 1), Placement::FlushAndGrow(64 << 10), "first upload");
        assert_eq!(place(0, 0, (64 << 10) + 1), Placement::FlushAndGrow(128 << 10));
        // At the cap a full buffer is flushed and reused, never grown.
        assert_eq!(place(3 << 20, 4 << 20, 2 << 20), Placement::FlushAndGrow(4 << 20));
        assert_eq!(place(0, 0, 4 << 20), Placement::FlushAndGrow(4 << 20));
        // A large inline image must not pin its staging memory while idle.
        assert_eq!(place(0, 4 << 20, (4 << 20) + 1), Placement::OneOff);
    }
}
