//! Host-visible coherent `HostBuffer<T>` and 2D `Texture` for Linux Vulkan.

use std::ffi::c_void;
use std::marker::PhantomData;
use std::sync::Arc;

use super::device::{Device, Queue, VulkanCore};
use super::ffi::*;
use crate::render::backend::TextureFormat;

pub(super) unsafe fn find_memory_type(
    props: &VkPhysicalDeviceMemoryProperties,
    type_bits: u32,
    required: VkFlags,
) -> u32 {
    for i in 0..props.memory_type_count {
        if (type_bits & (1 << i)) != 0 && (props.memory_types[i as usize].property_flags & required) == required {
            return i;
        }
    }
    0
}

pub(super) unsafe fn alloc_host_buffer(
    lib: &VkLib,
    device: VkDevice,
    mem_props: &VkPhysicalDeviceMemoryProperties,
    byte_len: usize,
    usage: VkFlags,
) -> (VkBuffer, VkDeviceMemory, *mut u8) {
    unsafe {
        #[repr(C)]
        struct BufInfo {
            s_type: u32,
            p_next: *const c_void,
            flags: u32,
            size: VkDeviceSize,
            usage: VkFlags,
            sharing_mode: u32,
            queue_family_index_count: u32,
            p_queue_family_indices: *const u32,
        }
        let b_info = BufInfo {
            s_type: VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            size: byte_len.max(16) as u64,
            usage,
            sharing_mode: 0,
            queue_family_index_count: 0,
            p_queue_family_indices: std::ptr::null(),
        };
        let mut buf = 0u64;
        (lib.vkCreateBuffer)(device, &b_info as *const _ as *const c_void, std::ptr::null(), &mut buf);
        let mut req = VkMemoryRequirements::default();
        (lib.vkGetBufferMemoryRequirements)(device, buf, &mut req);
        let mem_type = find_memory_type(
            mem_props,
            req.memory_type_bits,
            VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT,
        );
        #[repr(C)]
        struct MemAlloc {
            s_type: u32,
            p_next: *const c_void,
            allocation_size: VkDeviceSize,
            memory_type_index: u32,
        }
        let m_alloc = MemAlloc {
            s_type: VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
            p_next: std::ptr::null(),
            allocation_size: req.size,
            memory_type_index: mem_type,
        };
        let mut mem = 0u64;
        (lib.vkAllocateMemory)(device, &m_alloc as *const _ as *const c_void, std::ptr::null(), &mut mem);
        (lib.vkBindBufferMemory)(device, buf, mem, 0);
        let mut ptr: *mut c_void = std::ptr::null_mut();
        (lib.vkMapMemory)(device, mem, 0, req.size, 0, &mut ptr);
        std::ptr::write_bytes(ptr as *mut u8, 0, byte_len);
        (buf, mem, ptr as *mut u8)
    }
}

/// Host-visible coherent buffer (`VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT`).
pub struct HostBuffer<T: bytemuck::Pod> {
    vk: Option<Arc<VulkanCore>>,
    buf: VkBuffer,
    mem: VkDeviceMemory,
    ptr: *mut T,
    #[allow(dead_code)]
    fallback: Vec<T>,
    len: usize,
    cap: usize,
    _marker: PhantomData<T>,
}

unsafe impl<T: bytemuck::Pod + Send> Send for HostBuffer<T> {}
unsafe impl<T: bytemuck::Pod + Sync> Sync for HostBuffer<T> {}

impl<T: bytemuck::Pod> HostBuffer<T> {
    pub fn new(device: &Device, capacity: usize) -> Self {
        let cap = capacity.max(1);
        let elem_size = std::mem::size_of::<T>().max(1);
        let byte_len = cap * elem_size;
        if let Some(vk) = &device.vk {
            unsafe {
                let (buf, mem, ptr) = alloc_host_buffer(
                    &vk.lib,
                    vk.device,
                    &vk.mem_props,
                    byte_len,
                    VK_BUFFER_USAGE_VERTEX_BUFFER_BIT
                        | VK_BUFFER_USAGE_STORAGE_BUFFER_BIT
                        | VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT,
                );
                return Self {
                    vk: Some(vk.clone()),
                    buf,
                    mem,
                    ptr: ptr as *mut T,
                    fallback: Vec::new(),
                    len: 0,
                    cap,
                    _marker: PhantomData,
                };
            }
        }
        let mut fallback = vec![T::zeroed(); cap];
        let ptr = fallback.as_mut_ptr();
        Self {
            vk: None,
            buf: 0,
            mem: 0,
            ptr,
            fallback,
            len: 0,
            cap,
            _marker: PhantomData,
        }
    }

    #[cfg(test)]
    pub fn from_slice(device: &Device, data: &[T]) -> Self {
        let mut b = Self::new(device, data.len().max(1));
        b.len = data.len();
        if !data.is_empty() {
            b.as_mut_slice().copy_from_slice(data);
        }
        b
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn resize(&mut self, device: &Device, new_len: usize) -> bool {
        if new_len > self.cap || (self.cap > new_len.saturating_mul(2) && self.cap > 4096) {
            *self = Self::new(device, new_len.max(1));
            self.len = new_len;
            true
        } else {
            if new_len > self.len {
                unsafe {
                    std::ptr::write_bytes(self.ptr.add(self.len), 0, new_len - self.len);
                }
            }
            self.len = new_len;
            false
        }
    }

    #[inline]
    pub fn as_slice(&self) -> &[T] {
        if self.len == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
        }
    }

    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        if self.len == 0 {
            &mut []
        } else {
            unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
        }
    }

    #[inline]
    pub fn write(&mut self, device: &Device, data: &[T]) {
        if data.len() > self.cap {
            *self = Self::new(device, data.len().next_power_of_two());
        }
        self.len = data.len();
        if !data.is_empty() {
            self.as_mut_slice().copy_from_slice(data);
        }
    }

    #[inline]
    pub fn flush_range(&self, _queue: &Queue, _start: usize, _end: usize) {}

    #[inline]
    pub fn raw_ptr(&self) -> u64 {
        self.buf
    }
}

impl<T: bytemuck::Pod> Drop for HostBuffer<T> {
    fn drop(&mut self) {
        if let Some(vk) = &self.vk {
            unsafe {
                (vk.lib.vkDeviceWaitIdle)(vk.device);
                if self.buf != 0 {
                    (vk.lib.vkDestroyBuffer)(vk.device, self.buf, std::ptr::null());
                }
                if self.mem != 0 {
                    (vk.lib.vkFreeMemory)(vk.device, self.mem, std::ptr::null());
                }
            }
        }
    }
}

struct TextureInner {
    vk: Option<Arc<VulkanCore>>,
    image: VkImage,
    mem: VkDeviceMemory,
    view: VkImageView,
}

impl Drop for TextureInner {
    fn drop(&mut self) {
        if let Some(vk) = &self.vk {
            unsafe {
                (vk.lib.vkDeviceWaitIdle)(vk.device);
                if self.view != 0 {
                    (vk.lib.vkDestroyImageView)(vk.device, self.view, std::ptr::null());
                }
                if self.image != 0 {
                    (vk.lib.vkDestroyImage)(vk.device, self.image, std::ptr::null());
                }
                if self.mem != 0 {
                    (vk.lib.vkFreeMemory)(vk.device, self.mem, std::ptr::null());
                }
            }
        }
    }
}

#[derive(Clone)]
pub struct Texture {
    inner: Arc<TextureInner>,
    #[cfg(test)]
    pub width: u32,
    #[cfg(test)]
    pub height: u32,
}

impl Texture {
    pub fn new(device: &Device, width: u32, height: u32, format: TextureFormat, render_target: bool) -> Self {
        let w = width.max(1);
        let h = height.max(1);
        if let Some(vk) = &device.vk {
            unsafe {
                #[repr(C)]
                struct ImgInfo {
                    s_type: u32,
                    p_next: *const c_void,
                    flags: u32,
                    image_type: u32,
                    format: u32,
                    extent: VkExtent3D,
                    mip_levels: u32,
                    array_layers: u32,
                    samples: u32,
                    tiling: u32,
                    usage: VkFlags,
                    sharing_mode: u32,
                    queue_family_index_count: u32,
                    p_queue_family_indices: *const u32,
                    initial_layout: u32,
                }
                let mut usage = VK_IMAGE_USAGE_SAMPLED_BIT | VK_IMAGE_USAGE_TRANSFER_DST_BIT;
                if render_target {
                    usage |= VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT | VK_IMAGE_USAGE_TRANSFER_SRC_BIT;
                }
                let img_info = ImgInfo {
                    s_type: VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO,
                    p_next: std::ptr::null(),
                    flags: 0,
                    image_type: 1,
                    format: vk_format(format),
                    extent: VkExtent3D { width: w, height: h, depth: 1 },
                    mip_levels: 1,
                    array_layers: 1,
                    samples: 1,
                    tiling: 0,
                    usage,
                    sharing_mode: 0,
                    queue_family_index_count: 0,
                    p_queue_family_indices: std::ptr::null(),
                    initial_layout: VK_IMAGE_LAYOUT_UNDEFINED,
                };
                let mut image = 0u64;
                (vk.lib.vkCreateImage)(vk.device, &img_info as *const _ as *const c_void, std::ptr::null(), &mut image);
                let mut req = VkMemoryRequirements::default();
                (vk.lib.vkGetImageMemoryRequirements)(vk.device, image, &mut req);
                let mem_type = find_memory_type(&vk.mem_props, req.memory_type_bits, 0);
                #[repr(C)]
                struct MemAlloc {
                    s_type: u32,
                    p_next: *const c_void,
                    allocation_size: VkDeviceSize,
                    memory_type_index: u32,
                }
                let m_alloc = MemAlloc {
                    s_type: VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
                    p_next: std::ptr::null(),
                    allocation_size: req.size,
                    memory_type_index: mem_type,
                };
                let mut mem = 0u64;
                (vk.lib.vkAllocateMemory)(vk.device, &m_alloc as *const _ as *const c_void, std::ptr::null(), &mut mem);
                (vk.lib.vkBindImageMemory)(vk.device, image, mem, 0);

                #[repr(C)]
                struct ViewInfo {
                    s_type: u32,
                    p_next: *const c_void,
                    flags: u32,
                    image: VkImage,
                    view_type: u32,
                    format: u32,
                    components: [u32; 4],
                    subresource_range: [u32; 5],
                }
                let v_info = ViewInfo {
                    s_type: VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO,
                    p_next: std::ptr::null(),
                    flags: 0,
                    image,
                    view_type: 1,
                    format: vk_format(format),
                    components: [0; 4],
                    subresource_range: [1, 0, 1, 0, 1],
                };
                let mut view = 0u64;
                (vk.lib.vkCreateImageView)(vk.device, &v_info as *const _ as *const c_void, std::ptr::null(), &mut view);

                return Self {
                    inner: Arc::new(TextureInner {
                        vk: Some(vk.clone()),
                        image,
                        mem,
                        view,
                    }),
                    #[cfg(test)]
                    width: w,
                    #[cfg(test)]
                    height: h,
                };
            }
        }
        Self {
            inner: Arc::new(TextureInner {
                vk: None,
                image: 0,
                mem: 0,
                view: 0,
            }),
            #[cfg(test)]
            width: w,
            #[cfg(test)]
            height: h,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn write_rect(&self, _queue: &Queue, x: u32, y: u32, width: u32, height: u32, data: &[u8], _bytes_per_row: u32) {
        if width == 0 || height == 0 || data.is_empty() {
            return;
        }
        let Some(vk) = &self.inner.vk else { return };
        unsafe {
            let (stg_buf, stg_mem, stg_ptr) = alloc_host_buffer(
                &vk.lib,
                vk.device,
                &vk.mem_props,
                data.len(),
                VK_BUFFER_USAGE_TRANSFER_SRC_BIT,
            );
            std::ptr::copy_nonoverlapping(data.as_ptr(), stg_ptr, data.len());

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
            let to_dst = ImgBarrier {
                s_type: VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER,
                p_next: std::ptr::null(),
                src_access_mask: 0,
                dst_access_mask: VK_ACCESS_TRANSFER_WRITE_BIT,
                old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
                new_layout: VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                src_queue_family_index: u32::MAX,
                dst_queue_family_index: u32::MAX,
                image: self.inner.image,
                subresource_range: [1, 0, 1, 0, 1],
            };
            (vk.lib.vkCmdPipelineBarrier)(
                vk.upload_cmd,
                VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
                VK_PIPELINE_STAGE_TRANSFER_BIT,
                0,
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
                1,
                &to_dst as *const _ as *const c_void,
            );

            #[repr(C)]
            struct BufImgCopy {
                buffer_offset: VkDeviceSize,
                buffer_row_length: u32,
                buffer_image_height: u32,
                image_subresource: [u32; 4],
                image_offset: VkOffset3D,
                image_extent: VkExtent3D,
            }
            let region = BufImgCopy {
                buffer_offset: 0,
                buffer_row_length: 0,
                buffer_image_height: 0,
                image_subresource: [1, 0, 0, 1],
                image_offset: VkOffset3D { x: x as i32, y: y as i32, z: 0 },
                image_extent: VkExtent3D { width, height, depth: 1 },
            };
            (vk.lib.vkCmdCopyBufferToImage)(
                vk.upload_cmd,
                stg_buf,
                self.inner.image,
                VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                1,
                &region as *const _ as *const c_void,
            );

            let to_read = ImgBarrier {
                s_type: VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER,
                p_next: std::ptr::null(),
                src_access_mask: VK_ACCESS_TRANSFER_WRITE_BIT,
                dst_access_mask: VK_ACCESS_SHADER_READ_BIT,
                old_layout: VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                src_queue_family_index: u32::MAX,
                dst_queue_family_index: u32::MAX,
                image: self.inner.image,
                subresource_range: [1, 0, 1, 0, 1],
            };
            (vk.lib.vkCmdPipelineBarrier)(
                vk.upload_cmd,
                VK_PIPELINE_STAGE_TRANSFER_BIT,
                VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
                0,
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
                1,
                &to_read as *const _ as *const c_void,
            );

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
            let s_info = SubmitInfo {
                s_type: VK_STRUCTURE_TYPE_SUBMIT_INFO,
                p_next: std::ptr::null(),
                wait_semaphore_count: 0,
                p_wait_semaphores: std::ptr::null(),
                p_wait_dst_stage_mask: std::ptr::null(),
                command_buffer_count: 1,
                p_command_buffers: &vk.upload_cmd,
                signal_semaphore_count: 0,
                p_signal_semaphores: std::ptr::null(),
            };
            (vk.lib.vkQueueSubmit)(vk.queue, 1, &s_info as *const _ as *const c_void, 0);
            (vk.lib.vkQueueWaitIdle)(vk.queue);
            (vk.lib.vkDestroyBuffer)(vk.device, stg_buf, std::ptr::null());
            (vk.lib.vkFreeMemory)(vk.device, stg_mem, std::ptr::null());
        }
    }

    #[cfg(test)]
    pub fn read_pixels(&self, _dst: &mut [u8], _bytes_per_row: u32) {}

    #[inline]
    pub fn raw_ptr(&self) -> RawTexHandle {
        self.inner.view
    }
}
