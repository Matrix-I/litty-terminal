//! Host-visible coherent `HostBuffer<T>` and 2D `Texture` for Linux Vulkan.

use std::ffi::c_void;
use std::marker::PhantomData;
use std::sync::Arc;

use super::device::{Device, Queue, VulkanCore};
use super::ffi::*;
use super::upload::image_barrier;
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

/// A mapped host-visible coherent buffer of at least `byte_len` zeroed
/// bytes, or `None` (reported, nothing leaked) when it cannot be made.
pub(super) unsafe fn alloc_host_buffer(
    lib: &VkLib,
    device: VkDevice,
    mem_props: &VkPhysicalDeviceMemoryProperties,
    byte_len: usize,
    usage: VkFlags,
) -> Option<(VkBuffer, VkDeviceMemory, *mut u8)> {
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
        if !vk_ok((lib.vkCreateBuffer)(device, &b_info as *const _ as *const c_void, std::ptr::null(), &mut buf), "vkCreateBuffer") {
            return None;
        }
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
        let mut ptr: *mut c_void = std::ptr::null_mut();
        let ready = vk_ok((lib.vkAllocateMemory)(device, &m_alloc as *const _ as *const c_void, std::ptr::null(), &mut mem), "vkAllocateMemory")
            && vk_ok((lib.vkBindBufferMemory)(device, buf, mem, 0), "vkBindBufferMemory")
            && vk_ok((lib.vkMapMemory)(device, mem, 0, req.size, 0, &mut ptr), "vkMapMemory")
            && !ptr.is_null();
        if !ready {
            (lib.vkDestroyBuffer)(device, buf, std::ptr::null());
            if mem != 0 {
                (lib.vkFreeMemory)(device, mem, std::ptr::null());
            }
            return None;
        }
        std::ptr::write_bytes(ptr as *mut u8, 0, byte_len);
        Some((buf, mem, ptr as *mut u8))
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
        // Without GPU memory the contents live in `fallback`, with `buf` 0:
        // draws skip a buffer they cannot bind.
        if let Some(vk) = &device.vk {
            let usage = VK_BUFFER_USAGE_VERTEX_BUFFER_BIT | VK_BUFFER_USAGE_STORAGE_BUFFER_BIT | VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT;
            // SAFETY: `vk` is a live device and its function table.
            if let Some((buf, mem, ptr)) = unsafe { alloc_host_buffer(&vk.lib, vk.device, &vk.mem_props, byte_len, usage) } {
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

/// Where an upload's first barrier starts: `(old_layout, src_access, src_stage)`.
/// A ready texture is sampled between uploads and keeps its contents; one that
/// was never made ready starts from `UNDEFINED`.
fn upload_source(ready: bool) -> (u32, VkFlags, VkFlags) {
    if ready {
        (VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, VK_ACCESS_SHADER_READ_BIT, VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT)
    } else {
        (VK_IMAGE_LAYOUT_UNDEFINED, 0, VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT)
    }
}

/// `VkBufferImageCopy` for the color aspect of mip level 0, layer 0.
#[repr(C)]
struct BufImgCopy {
    buffer_offset: VkDeviceSize,
    buffer_row_length: u32,
    buffer_image_height: u32,
    image_subresource: [u32; 4],
    image_offset: VkOffset3D,
    image_extent: VkExtent3D,
}

/// A framebuffer for drawing into the render target `view`, registered with
/// the offscreen render passes of `format`; 0 when it could not be created.
unsafe fn create_target_framebuffer(
    vk: &VulkanCore,
    view: VkImageView,
    format: TextureFormat,
    width: u32,
    height: u32,
) -> VkFramebuffer {
    #[repr(C)]
    struct FbInfo {
        s_type: u32,
        p_next: *const c_void,
        flags: u32,
        render_pass: VkRenderPass,
        attachment_count: u32,
        p_attachments: *const VkImageView,
        width: u32,
        height: u32,
        layers: u32,
    }
    let (clear_pass, load_pass) = super::device::offscreen_render_passes(vk, format);
    let info = FbInfo {
        s_type: VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO,
        p_next: std::ptr::null(),
        flags: 0,
        render_pass: clear_pass,
        attachment_count: 1,
        p_attachments: &view,
        width,
        height,
        layers: 1,
    };
    let mut fb = 0u64;
    // SAFETY: `info` and `view` outlive the call; `view` belongs to `vk.device`.
    let created = unsafe { (vk.lib.vkCreateFramebuffer)(vk.device, &info as *const _ as *const c_void, std::ptr::null(), &mut fb) };
    if created != VK_SUCCESS || fb == 0 {
        eprintln!("litty: could not create a {width}x{height} render target");
        return 0;
    }
    super::surface::register_framebuffer(
        fb,
        super::surface::FramebufferTarget { width, height, clear_pass, load_pass },
    );
    fb
}

/// Allocate and fill the image pipeline's descriptor set for `view`: the
/// shared image uniforms, the view and the linear sampler. Written once, so
/// it is never updated while a recorded frame still refers to it. Returns 0
/// when the pool is exhausted (the image is then skipped, not drawn wrongly).
unsafe fn alloc_image_dset(vk: &VulkanCore, view: VkImageView) -> VkDescriptorSet {
    unsafe {
        #[repr(C)]
        struct DsetAlloc {
            s_type: u32,
            p_next: *const c_void,
            descriptor_pool: VkDescriptorPool,
            descriptor_set_count: u32,
            p_set_layouts: *const VkDescriptorSetLayout,
        }
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
        let alloc = DsetAlloc {
            s_type: VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO,
            p_next: std::ptr::null(),
            descriptor_pool: vk.image_desc_pool,
            descriptor_set_count: 1,
            p_set_layouts: &vk.image_dset_layout,
        };
        let mut dset = 0u64;
        {
            let _pool = vk.image_pool_lock.lock();
            if (vk.lib.vkAllocateDescriptorSets)(vk.device, &alloc as *const _ as *const c_void, &mut dset) != VK_SUCCESS {
                return 0;
            }
        }
        let uniforms = BufDesc { buffer: vk.image_ubuf.0, offset: 0, range: u64::MAX };
        let texture = ImgDesc { sampler: 0, image_view: view, image_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL };
        let sampler = ImgDesc { sampler: vk.linear_sampler, image_view: 0, image_layout: 0 };
        let write = |binding: u32, ty: u32, image: *const ImgDesc, buffer: *const BufDesc| WriteDesc {
            s_type: VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET,
            p_next: std::ptr::null(),
            dst_set: dset,
            dst_binding: binding,
            dst_array_element: 0,
            descriptor_count: 1,
            descriptor_type: ty,
            p_image_info: image,
            p_buffer_info: buffer,
            p_texel_buffer_view: std::ptr::null(),
        };
        let writes = [
            write(0, VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER, std::ptr::null(), &uniforms),
            write(1, VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE, &texture, std::ptr::null()),
            write(2, VK_DESCRIPTOR_TYPE_SAMPLER, &sampler, std::ptr::null()),
        ];
        (vk.lib.vkUpdateDescriptorSets)(vk.device, writes.len() as u32, writes.as_ptr() as *const c_void, 0, std::ptr::null());
        dset
    }
}

struct TextureInner {
    vk: Option<Arc<VulkanCore>>,
    image: VkImage,
    mem: VkDeviceMemory,
    view: VkImageView,
    /// In `SHADER_READ_ONLY_OPTIMAL` between uploads (every sampled texture).
    ready: bool,
    /// Descriptor set for drawing this texture as an inline image, or 0.
    image_dset: VkDescriptorSet,
    /// Framebuffer of a render target (registered with its render passes),
    /// or 0 for a sampled texture.
    framebuffer: VkFramebuffer,
    /// Size of the image, for `read_pixels`.
    #[cfg_attr(not(test), allow(dead_code))]
    width: u32,
    #[cfg_attr(not(test), allow(dead_code))]
    height: u32,
}

impl Drop for TextureInner {
    fn drop(&mut self) {
        if let Some(vk) = &self.vk {
            unsafe {
                // The pending upload batch may copy into this image.
                vk.uploads.lock().flush(vk);
                (vk.lib.vkDeviceWaitIdle)(vk.device);
                if self.framebuffer != 0 {
                    super::surface::forget_framebuffer(self.framebuffer);
                    (vk.lib.vkDestroyFramebuffer)(vk.device, self.framebuffer, std::ptr::null());
                }
                if self.image_dset != 0 {
                    let _pool = vk.image_pool_lock.lock();
                    (vk.lib.vkFreeDescriptorSets)(vk.device, vk.image_desc_pool, 1, &self.image_dset);
                }
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
                if !vk_ok((vk.lib.vkCreateImage)(vk.device, &img_info as *const _ as *const c_void, std::ptr::null(), &mut image), "vkCreateImage") {
                    return Self::unusable(w, h);
                }
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
                // Undo a partly created texture; it is then drawn as nothing.
                let discard = |mem: VkDeviceMemory, view: VkImageView| {
                    if view != 0 {
                        (vk.lib.vkDestroyImageView)(vk.device, view, std::ptr::null());
                    }
                    (vk.lib.vkDestroyImage)(vk.device, image, std::ptr::null());
                    if mem != 0 {
                        (vk.lib.vkFreeMemory)(vk.device, mem, std::ptr::null());
                    }
                    Self::unusable(w, h)
                };
                if !(vk_ok((vk.lib.vkAllocateMemory)(vk.device, &m_alloc as *const _ as *const c_void, std::ptr::null(), &mut mem), "vkAllocateMemory")
                    && vk_ok((vk.lib.vkBindImageMemory)(vk.device, image, mem, 0), "vkBindImageMemory"))
                {
                    return discard(mem, 0);
                }

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
                if !vk_ok((vk.lib.vkCreateImageView)(vk.device, &v_info as *const _ as *const c_void, std::ptr::null(), &mut view), "vkCreateImageView") {
                    return discard(mem, 0);
                }

                // A sampled texture may be bound before anything is uploaded
                // (the color atlas placeholder): give it a valid layout now.
                let ready = !render_target;
                if ready {
                    // Recorded into the upload batch, which runs before any
                    // frame that could sample the texture.
                    let mut uploads = vk.uploads.lock();
                    let Some(cmd) = uploads.begin(vk) else { return discard(mem, view) };
                    image_barrier(
                        vk,
                        cmd,
                        image,
                        (VK_IMAGE_LAYOUT_UNDEFINED, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL),
                        (0, VK_ACCESS_SHADER_READ_BIT),
                        (VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT, VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT),
                    );
                }
                let image_dset = if ready { alloc_image_dset(vk, view) } else { 0 };
                let framebuffer = if render_target { create_target_framebuffer(vk, view, format, w, h) } else { 0 };
                if render_target && framebuffer == 0 {
                    return discard(mem, view);
                }
                return Self {
                    inner: Arc::new(TextureInner {
                        vk: Some(vk.clone()),
                        image,
                        mem,
                        view,
                        ready,
                        image_dset,
                        framebuffer,
                        width: w,
                        height: h,
                    }),
                };
            }
        }
        Self::unusable(w, h)
    }

    /// A texture without GPU objects (no device, or creating them failed):
    /// uploads are dropped and draws using it are skipped.
    fn unusable(width: u32, height: u32) -> Self {
        Self {
            inner: Arc::new(TextureInner {
                vk: None,
                image: 0,
                mem: 0,
                view: 0,
                ready: false,
                image_dset: 0,
                framebuffer: 0,
                width,
                height,
            }),
        }
    }

    /// Copy `data` (`width` x `height` tightly packed texels) to `(x, y)`.
    /// The copy is recorded into the upload batch and runs ahead of the next
    /// frame (see `upload`), so callers may upload while a frame records.
    #[allow(clippy::too_many_arguments)]
    pub fn write_rect(&self, _queue: &Queue, x: u32, y: u32, width: u32, height: u32, data: &[u8], _bytes_per_row: u32) {
        if width == 0 || height == 0 || data.is_empty() {
            return;
        }
        let Some(vk) = &self.inner.vk else { return };
        unsafe {
            let mut uploads = vk.uploads.lock();
            let Some((buf, offset)) = uploads.stage(vk, data) else { return };
            let Some(cmd) = uploads.begin(vk) else { return };
            let (old_layout, src_access, src_stage) = upload_source(self.inner.ready);
            image_barrier(
                vk,
                cmd,
                self.inner.image,
                (old_layout, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL),
                (src_access, VK_ACCESS_TRANSFER_WRITE_BIT),
                (src_stage, VK_PIPELINE_STAGE_TRANSFER_BIT),
            );
            let region = BufImgCopy {
                buffer_offset: offset,
                buffer_row_length: 0,
                buffer_image_height: 0,
                image_subresource: [1, 0, 0, 1],
                image_offset: VkOffset3D { x: x as i32, y: y as i32, z: 0 },
                image_extent: VkExtent3D { width, height, depth: 1 },
            };
            (vk.lib.vkCmdCopyBufferToImage)(
                cmd,
                buf,
                self.inner.image,
                VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                1,
                &region as *const _ as *const c_void,
            );
            image_barrier(
                vk,
                cmd,
                self.inner.image,
                (VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL),
                (VK_ACCESS_TRANSFER_WRITE_BIT, VK_ACCESS_SHADER_READ_BIT),
                (VK_PIPELINE_STAGE_TRANSFER_BIT, VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT),
            );
        }
    }

    /// Copy a render target's pixels (tightly packed 4-byte texels) into
    /// `dst`, `bytes_per_row` apart. It must have been drawn into since it
    /// was created; its render pass leaves it ready to be copied from.
    #[cfg(test)]
    pub fn read_pixels(&self, dst: &mut [u8], bytes_per_row: u32) {
        use super::upload::memory_barrier;
        let Some(vk) = &self.inner.vk else { return };
        assert_ne!(self.inner.framebuffer, 0, "only render targets can be read back");
        let (w, h) = (self.inner.width, self.inner.height);
        let row = w as usize * 4;
        let len = row * h as usize;
        unsafe {
            let (buf, mem, ptr) = alloc_host_buffer(&vk.lib, vk.device, &vk.mem_props, len, VK_BUFFER_USAGE_TRANSFER_DST_BIT)
                .expect("read-back buffer");
            let mut uploads = vk.uploads.lock();
            let cmd = uploads.begin(vk).expect("upload batch");
            // Make the render pass's writes visible to the copy.
            image_barrier(
                vk,
                cmd,
                self.inner.image,
                (VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL),
                (VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT, VK_ACCESS_TRANSFER_READ_BIT),
                (VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT, VK_PIPELINE_STAGE_TRANSFER_BIT),
            );
            let region = BufImgCopy {
                buffer_offset: 0,
                buffer_row_length: 0,
                buffer_image_height: 0,
                image_subresource: [1, 0, 0, 1],
                image_offset: VkOffset3D { x: 0, y: 0, z: 0 },
                image_extent: VkExtent3D { width: w, height: h, depth: 1 },
            };
            (vk.lib.vkCmdCopyImageToBuffer)(
                cmd,
                self.inner.image,
                VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                buf,
                1,
                &region as *const _ as *const c_void,
            );
            memory_barrier(
                vk,
                cmd,
                (VK_ACCESS_TRANSFER_WRITE_BIT, VK_ACCESS_HOST_READ_BIT),
                (VK_PIPELINE_STAGE_TRANSFER_BIT, VK_PIPELINE_STAGE_HOST_BIT),
            );
            uploads.flush(vk);
            let src = std::slice::from_raw_parts(ptr, len);
            for (y, line) in src.chunks_exact(row).enumerate() {
                let start = y * bytes_per_row as usize;
                if let Some(out) = dst.get_mut(start..start + row) {
                    out.copy_from_slice(line);
                }
            }
            (vk.lib.vkDestroyBuffer)(vk.device, buf, std::ptr::null());
            (vk.lib.vkFreeMemory)(vk.device, mem, std::ptr::null());
        }
    }

    /// The handle the renderer passes around for this texture: for a render
    /// target its framebuffer (what `CommandEncoder::begin_render_pass`
    /// takes), otherwise its image view (what draws sample).
    #[inline]
    pub fn raw_ptr(&self) -> RawTexHandle {
        if self.inner.framebuffer != 0 { self.inner.framebuffer } else { self.inner.view }
    }

    /// Descriptor set binding this texture to the image pipeline; 0 when it
    /// has none (render target, no GPU, or the pool was exhausted).
    #[inline]
    pub(super) fn image_dset(&self) -> VkDescriptorSet {
        self.inner.image_dset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uploads_keep_the_contents_of_a_ready_texture() {
        // A glyph upload writes one cell: starting from UNDEFINED would let the
        // driver discard every glyph already in the atlas.
        assert_eq!(
            upload_source(true),
            (VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, VK_ACCESS_SHADER_READ_BIT, VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT)
        );
        // A render target has no contents worth keeping before its first upload.
        assert_eq!(upload_source(false), (VK_IMAGE_LAYOUT_UNDEFINED, 0, VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT));
    }

    #[test]
    fn sampled_textures_own_an_image_descriptor_set() {
        let (device, _queue) = super::super::device::shared_device_and_queue();
        let sampled = Texture::new(&device, 4, 4, TextureFormat::Rgba8Unorm, false);
        // With a GPU every sampled texture can be drawn as an inline image;
        // without one (headless test machines) there is nothing to bind.
        assert_eq!(sampled.image_dset() != 0, device.vk.is_some());
        let target = Texture::new(&device, 4, 4, TextureFormat::Bgra8Unorm, true);
        assert_eq!(target.image_dset(), 0, "render targets are never drawn as images");
    }

    /// `image_layer::Quad`: rect (px), uv.
    #[repr(C)]
    #[derive(Copy, Clone, Default, bytemuck::Pod, bytemuck::Zeroable)]
    struct Quad {
        rect: [f32; 4],
        uv: [f32; 4],
    }

    /// `image_layer::Uniforms`.
    #[repr(C)]
    #[derive(Copy, Clone, Default, bytemuck::Pod, bytemuck::Zeroable)]
    struct ImageUniforms {
        screen: [f32; 2],
        flip_y: u32,
        _pad: u32,
    }

    /// Draw `texture` over a 16x16 render target and return its pixels.
    /// `upload` runs before the frame starts (`before`) or while it records.
    fn draw_uploaded_image(upload_while_recording: bool) -> Option<Vec<u8>> {
        use super::super::pass::CommandEncoder;
        let (device, queue) = super::super::device::shared_device_and_queue();
        device.vk.as_ref()?;
        let format = TextureFormat::Rgba8Unorm;
        let pipes = super::super::device::shared_pipelines(&device, format);
        let target = Texture::new(&device, 16, 16, format, true);
        let image = Texture::new(&device, 4, 4, format, false);
        let red = [255u8, 0, 0, 255].repeat(16);
        let quads = HostBuffer::from_slice(&device, &[Quad { rect: [0.0, 0.0, 16.0, 16.0], uv: [0.0, 0.0, 1.0, 1.0] }]);
        let uniforms = ImageUniforms { screen: [16.0, 16.0], ..Default::default() };
        if !upload_while_recording {
            image.write_rect(&queue, 0, 0, 4, 4, &red, 16);
        }
        let mut encoder = CommandEncoder::new(&queue);
        if upload_while_recording {
            image.write_rect(&queue, 0, 0, 4, 4, &red, 16);
        }
        {
            let mut pass = encoder.begin_render_pass(target.raw_ptr(), Some([0.0, 0.0, 0.0, 1.0]));
            pass.draw_image_quad(&pipes.image, &quads, &uniforms, &image, device.linear_sampler(), 0);
        }
        encoder.submit_and_wait();
        let mut pixels = vec![0u8; 16 * 16 * 4];
        target.read_pixels(&mut pixels, 16 * 4);
        Some(pixels)
    }

    /// An inline image uploaded for a frame is drawn by that frame, whether
    /// the upload happens before the frame starts recording or during it.
    #[test]
    #[ignore = "requires a Vulkan device"]
    fn an_uploaded_image_is_drawn_in_the_same_frame() {
        for while_recording in [false, true] {
            let Some(pixels) = draw_uploaded_image(while_recording) else { return };
            assert_eq!(&pixels[(8 * 16 + 8) * 4..][..4], &[255, 0, 0, 255], "upload while recording: {while_recording}");
        }
    }

    /// Not a check: prints how long a frame with many new glyphs takes
    /// (`cargo test --bin litty glyph_upload_cost -- --ignored --nocapture`).
    #[test]
    #[ignore = "benchmark; requires a Vulkan device"]
    fn glyph_upload_cost() {
        use super::super::pass::CommandEncoder;
        let (device, queue) = super::super::device::shared_device_and_queue();
        if device.vk.is_none() {
            return;
        }
        let atlas = Texture::new(&device, 1024, 1024, TextureFormat::R8Unorm, false);
        let target = Texture::new(&device, 64, 64, TextureFormat::Rgba8Unorm, true);
        let glyph = [0x80u8; 16 * 32];
        for glyphs in [1u32, 64, 1024] {
            let mut best = std::time::Duration::MAX;
            for _ in 0..5 {
                let start = std::time::Instant::now();
                for g in 0..glyphs {
                    atlas.write_rect(&queue, (g % 64) * 16, (g / 64) * 32, 16, 32, &glyph, 16);
                }
                let mut encoder = CommandEncoder::new(&queue);
                drop(encoder.begin_render_pass(target.raw_ptr(), Some([0.0; 4])));
                encoder.submit_and_wait();
                best = best.min(start.elapsed());
            }
            eprintln!("glyph_upload_cost: {glyphs:>4} new glyphs + 1 frame: {:.3} ms (best of 5)", best.as_secs_f64() * 1e3);
        }
    }
}
