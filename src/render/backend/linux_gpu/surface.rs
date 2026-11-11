//! Wayland / X11 `VkSurfaceKHR` and `VkSwapchainKHR` (`VK_PRESENT_MODE_FIFO_KHR`, `minImageCount = 2`).

use std::ffi::c_void;
use std::sync::Arc;

use crate::platform::gui::{RawHandle, Window};

use super::device::{shared_pipelines, Device, Queue, VulkanCore};
use super::ffi::*;
use crate::render::backend::{SurfaceConfig, TextureFormat};

/// Pixel size of every live swapchain framebuffer. A render pass only gets
/// the framebuffer handle, and Vulkan needs its extent for the render area,
/// viewport and scissor (all dynamic in our pipelines). A handful of entries
/// at most, so a linear scan without allocation on the render path.
static FB_EXTENTS: parking_lot::Mutex<Vec<(VkFramebuffer, u32, u32)>> = parking_lot::Mutex::new(Vec::new());

fn register_framebuffer(fb: VkFramebuffer, width: u32, height: u32) {
    let mut guard = FB_EXTENTS.lock();
    guard.retain(|e| e.0 != fb);
    guard.push((fb, width, height));
}

fn forget_framebuffer(fb: VkFramebuffer) {
    FB_EXTENTS.lock().retain(|e| e.0 != fb);
}

/// Extent of a swapchain framebuffer, or `None` for an unknown handle.
pub(super) fn framebuffer_extent(fb: VkFramebuffer) -> Option<(u32, u32)> {
    FB_EXTENTS.lock().iter().find(|e| e.0 == fb).map(|e| (e.1, e.2))
}

/// The extent a new swapchain must use, or `None` when the surface has no
/// area (minimised, or its capabilities could not be read). A surface with a
/// fixed size (X11) dictates it; `0xFFFFFFFF` (Wayland) leaves it to the
/// request, clamped to the surface's limits.
fn swapchain_extent(caps: &VkSurfaceCapabilitiesKHR, width: u32, height: u32) -> Option<VkExtent2D> {
    let current = caps.current_extent;
    if current.width != u32::MAX {
        return (current.width > 0 && current.height > 0).then_some(current);
    }
    let clamp = |v: u32, lo: u32, hi: u32| {
        let lo = lo.max(1);
        if hi >= lo { v.clamp(lo, hi) } else { v.max(lo) }
    };
    Some(VkExtent2D {
        width: clamp(width, caps.min_image_extent.width, caps.max_image_extent.width),
        height: clamp(height, caps.min_image_extent.height, caps.max_image_extent.height),
    })
}

pub struct Surface {
    vk: Option<Arc<VulkanCore>>,
    surface_khr: VkSurfaceKHR,
    swapchain: VkSwapchainKHR,
    views: Vec<VkImageView>,
    framebuffers: Vec<VkFramebuffer>,
    image_avail_sem: VkSemaphore,
    render_done_sem: VkSemaphore,
    fence: VkFence,
    width: u32,
    height: u32,
}

unsafe impl Send for Surface {}
unsafe impl Sync for Surface {}

pub struct SurfaceFrame {
    pub image_index: u32,
    pub framebuffer: VkFramebuffer,
    pub swapchain: VkSwapchainKHR,
    pub render_done_sem: VkSemaphore,
    pub image_avail_sem: VkSemaphore,
    pub fence: VkFence,
}

impl SurfaceFrame {
    #[inline]
    pub fn texture_ptr(&self) -> RawTexHandle {
        self.framebuffer
    }
}

impl Surface {
    pub fn new(window: &Window, device: &Device) -> Self {
        let mut surface_khr = 0u64;
        let mut image_avail_sem = 0u64;
        let mut render_done_sem = 0u64;
        let mut fence = 0u64;

        if let Some(vk) = &device.vk {
            unsafe {
                if let Some(handle) = window.raw_handle() {
                    match handle {
                        RawHandle::Wayland { display, surface } => {
                            if let Some(create_wl) = vk.lib.vkCreateWaylandSurfaceKHR {
                                #[repr(C)]
                                struct WlInfo {
                                    s_type: u32,
                                    p_next: *const c_void,
                                    flags: u32,
                                    display: *mut c_void,
                                    surface: *mut c_void,
                                }
                                let info = WlInfo {
                                    s_type: VK_STRUCTURE_TYPE_WAYLAND_SURFACE_CREATE_INFO_KHR,
                                    p_next: std::ptr::null(),
                                    flags: 0,
                                    display,
                                    surface,
                                };
                                create_wl(vk.instance, &info as *const _ as *const c_void, std::ptr::null(), &mut surface_khr);
                            }
                        }
                        RawHandle::Xlib { display, window } => {
                            if let Some(create_xlib) = vk.lib.vkCreateXlibSurfaceKHR {
                                #[repr(C)]
                                struct XlibInfo {
                                    s_type: u32,
                                    p_next: *const c_void,
                                    flags: u32,
                                    dpy: *mut c_void,
                                    window: libc::c_ulong,
                                }
                                let info = XlibInfo {
                                    s_type: VK_STRUCTURE_TYPE_XLIB_SURFACE_CREATE_INFO_KHR,
                                    p_next: std::ptr::null(),
                                    flags: 0,
                                    dpy: display,
                                    window,
                                };
                                create_xlib(vk.instance, &info as *const _ as *const c_void, std::ptr::null(), &mut surface_khr);
                            }
                        }
                        RawHandle::Xcb { connection, window } => {
                            if let Some(create_xcb) = vk.lib.vkCreateXcbSurfaceKHR {
                                #[repr(C)]
                                struct XcbInfo {
                                    s_type: u32,
                                    p_next: *const c_void,
                                    flags: u32,
                                    connection: *mut c_void,
                                    window: u32,
                                }
                                let info = XcbInfo {
                                    s_type: VK_STRUCTURE_TYPE_XCB_SURFACE_CREATE_INFO_KHR,
                                    p_next: std::ptr::null(),
                                    flags: 0,
                                    connection,
                                    window,
                                };
                                create_xcb(vk.instance, &info as *const _ as *const c_void, std::ptr::null(), &mut surface_khr);
                            }
                        }
                        _ => {}
                    }
                }

                #[repr(C)]
                struct SemInfo {
                    s_type: u32,
                    p_next: *const c_void,
                    flags: u32,
                }
                let sem_info = SemInfo { s_type: VK_STRUCTURE_TYPE_SEMAPHORE_CREATE_INFO, p_next: std::ptr::null(), flags: 0 };
                (vk.lib.vkCreateSemaphore)(vk.device, &sem_info as *const _ as *const c_void, std::ptr::null(), &mut image_avail_sem);
                (vk.lib.vkCreateSemaphore)(vk.device, &sem_info as *const _ as *const c_void, std::ptr::null(), &mut render_done_sem);

                #[repr(C)]
                struct FenceInfo {
                    s_type: u32,
                    p_next: *const c_void,
                    flags: u32,
                }
                let f_info = FenceInfo { s_type: VK_STRUCTURE_TYPE_FENCE_CREATE_INFO, p_next: std::ptr::null(), flags: 1 };
                (vk.lib.vkCreateFence)(vk.device, &f_info as *const _ as *const c_void, std::ptr::null(), &mut fence);
            }
        }

        Self {
            vk: device.vk.clone(),
            surface_khr,
            swapchain: 0,
            views: Vec::new(),
            framebuffers: Vec::new(),
            image_avail_sem,
            render_done_sem,
            fence,
            width: 1,
            height: 1,
        }
    }

    #[inline]
    pub fn default_format(&self) -> TextureFormat {
        TextureFormat::Bgra8Unorm
    }

    pub fn configure(&mut self, device: &Device, config: &SurfaceConfig) {
        self.width = config.width.max(1);
        self.height = config.height.max(1);
        let Some(vk) = &self.vk else { return };
        if self.surface_khr == 0 {
            return;
        }
        unsafe {
            (vk.lib.vkDeviceWaitIdle)(vk.device);
            for &fb in &self.framebuffers {
                forget_framebuffer(fb);
                (vk.lib.vkDestroyFramebuffer)(vk.device, fb, std::ptr::null());
            }
            self.framebuffers.clear();
            for &v in &self.views {
                (vk.lib.vkDestroyImageView)(vk.device, v, std::ptr::null());
            }
            self.views.clear();

            let mut caps = VkSurfaceCapabilitiesKHR::default();
            (vk.lib.vkGetPhysicalDeviceSurfaceCapabilitiesKHR)(vk.phys, self.surface_khr, &mut caps);
            let Some(extent) = swapchain_extent(&caps, self.width, self.height) else {
                // Nothing to present to: drop the old swapchain so no frame is acquired.
                if self.swapchain != 0 {
                    (vk.lib.vkDestroySwapchainKHR)(vk.device, self.swapchain, std::ptr::null());
                    self.swapchain = 0;
                }
                return;
            };
            // Framebuffers, render area and viewport all follow the real extent.
            self.width = extent.width;
            self.height = extent.height;

            let min_images = caps.min_image_count.max(2);
            let max_images = if caps.max_image_count > 0 { caps.max_image_count } else { min_images };
            let image_count = min_images.min(max_images);

            let alpha = if config.transparent {
                if (caps.supported_composite_alpha & VK_COMPOSITE_ALPHA_PRE_MULTIPLIED_BIT_KHR) != 0 {
                    VK_COMPOSITE_ALPHA_PRE_MULTIPLIED_BIT_KHR
                } else if (caps.supported_composite_alpha & VK_COMPOSITE_ALPHA_POST_MULTIPLIED_BIT_KHR) != 0 {
                    VK_COMPOSITE_ALPHA_POST_MULTIPLIED_BIT_KHR
                } else if (caps.supported_composite_alpha & VK_COMPOSITE_ALPHA_INHERIT_BIT_KHR) != 0 {
                    VK_COMPOSITE_ALPHA_INHERIT_BIT_KHR
                } else {
                    VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR
                }
            } else {
                VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR
            };

            #[repr(C)]
            struct SwapInfo {
                s_type: u32,
                p_next: *const c_void,
                flags: u32,
                surface: VkSurfaceKHR,
                min_image_count: u32,
                image_format: u32,
                image_color_space: u32,
                image_extent: VkExtent2D,
                image_array_layers: u32,
                image_usage: VkFlags,
                image_sharing_mode: u32,
                queue_family_index_count: u32,
                p_queue_family_indices: *const u32,
                pre_transform: VkFlags,
                composite_alpha: VkFlags,
                present_mode: u32,
                clipped: VkBool32,
                old_swapchain: VkSwapchainKHR,
            }
            let old_sc = self.swapchain;
            let sc_info = SwapInfo {
                s_type: VK_STRUCTURE_TYPE_SWAPCHAIN_CREATE_INFO_KHR,
                p_next: std::ptr::null(),
                flags: 0,
                surface: self.surface_khr,
                min_image_count: image_count,
                image_format: vk_format(config.format),
                image_color_space: 0,
                image_extent: extent,
                image_array_layers: 1,
                image_usage: VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT,
                image_sharing_mode: 0,
                queue_family_index_count: 0,
                p_queue_family_indices: std::ptr::null(),
                pre_transform: if caps.current_transform != 0 { caps.current_transform } else { 1 },
                composite_alpha: alpha,
                present_mode: VK_PRESENT_MODE_FIFO_KHR,
                clipped: 1,
                old_swapchain: old_sc,
            };
            let mut new_sc = 0u64;
            let created = (vk.lib.vkCreateSwapchainKHR)(vk.device, &sc_info as *const _ as *const c_void, std::ptr::null(), &mut new_sc) == VK_SUCCESS;
            // Destroy the old swapchain either way: it was replaced, or the create
            // failed and it must not be presented to again. Never keep its handle.
            if old_sc != 0 {
                (vk.lib.vkDestroySwapchainKHR)(vk.device, old_sc, std::ptr::null());
            }
            self.swapchain = if created { new_sc } else { 0 };
            if self.swapchain == 0 {
                eprintln!("litty: could not create a {}x{} swapchain", extent.width, extent.height);
                return;
            }

            let mut img_count = 0u32;
            (vk.lib.vkGetSwapchainImagesKHR)(vk.device, self.swapchain, &mut img_count, std::ptr::null_mut());
            let mut imgs = vec![0u64; img_count as usize];
            (vk.lib.vkGetSwapchainImagesKHR)(vk.device, self.swapchain, &mut img_count, imgs.as_mut_ptr());

            let pipes = shared_pipelines(device, config.format);
            for img in imgs {
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
                    image: img,
                    view_type: 1,
                    format: vk_format(config.format),
                    components: [0; 4],
                    subresource_range: [1, 0, 1, 0, 1],
                };
                let mut view = 0u64;
                (vk.lib.vkCreateImageView)(vk.device, &v_info as *const _ as *const c_void, std::ptr::null(), &mut view);
                self.views.push(view);

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
                let fb_info = FbInfo {
                    s_type: VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO,
                    p_next: std::ptr::null(),
                    flags: 0,
                    render_pass: pipes.render_pass_clear,
                    attachment_count: 1,
                    p_attachments: &view,
                    width: self.width,
                    height: self.height,
                    layers: 1,
                };
                let mut fb = 0u64;
                (vk.lib.vkCreateFramebuffer)(vk.device, &fb_info as *const _ as *const c_void, std::ptr::null(), &mut fb);
                register_framebuffer(fb, self.width, self.height);
                self.framebuffers.push(fb);
            }
        }
    }

    pub fn acquire_frame(&mut self) -> Option<SurfaceFrame> {
        let vk = self.vk.as_ref()?;
        if self.swapchain == 0 || self.framebuffers.is_empty() {
            return None;
        }
        unsafe {
            (vk.lib.vkWaitForFences)(vk.device, 1, &self.fence, 1, u64::MAX);
            let mut idx = 0u32;
            let res = (vk.lib.vkAcquireNextImageKHR)(
                vk.device,
                self.swapchain,
                u64::MAX,
                self.image_avail_sem,
                0,
                &mut idx,
            );
            if res != VK_SUCCESS && res != VK_SUBOPTIMAL_KHR {
                return None;
            }
            (vk.lib.vkResetFences)(vk.device, 1, &self.fence);
            Some(SurfaceFrame {
                image_index: idx,
                framebuffer: *self.framebuffers.get(idx as usize)?,
                swapchain: self.swapchain,
                render_done_sem: self.render_done_sem,
                image_avail_sem: self.image_avail_sem,
                fence: self.fence,
            })
        }
    }

    /// Wait up to `timeout` for the GPU to finish the last submitted frame (its
    /// fence is signaled by the submit); `false` on timeout, device loss or no
    /// device. Benchmarks only: the render loop never waits here.
    pub fn wait_gpu_idle(&self, timeout: std::time::Duration) -> bool {
        let Some(vk) = self.vk.as_ref() else { return false };
        if self.fence == 0 {
            return false;
        }
        let timeout_ns = u64::try_from(timeout.as_nanos()).unwrap_or(u64::MAX);
        // SAFETY: `fence` belongs to `vk.device` and lives as long as the surface.
        unsafe { (vk.lib.vkWaitForFences)(vk.device, 1, &self.fence, 1, timeout_ns) == VK_SUCCESS }
    }

    pub fn shrink_drawables(&mut self, _device: &Device, _queue: &Queue) {
        self.release_swapchain();
    }

    /// Destroy the framebuffers, image views and swapchain. The surface, the
    /// semaphores and the fence stay, so `configure` can build a new swapchain.
    fn release_swapchain(&mut self) {
        let Some(vk) = &self.vk else { return };
        unsafe {
            (vk.lib.vkDeviceWaitIdle)(vk.device);
            for &fb in &self.framebuffers {
                forget_framebuffer(fb);
                (vk.lib.vkDestroyFramebuffer)(vk.device, fb, std::ptr::null());
            }
            self.framebuffers.clear();
            for &v in &self.views {
                (vk.lib.vkDestroyImageView)(vk.device, v, std::ptr::null());
            }
            self.views.clear();
            if self.swapchain != 0 {
                (vk.lib.vkDestroySwapchainKHR)(vk.device, self.swapchain, std::ptr::null());
                self.swapchain = 0;
            }
        }
    }
}

/// A closed window must give back its swapchain images: they are the largest
/// allocation a window owns. The renderer is dropped before its window
/// (`app::State` field order), so the display connection is still open here.
impl Drop for Surface {
    fn drop(&mut self) {
        self.release_swapchain();
        let Some(vk) = &self.vk else { return };
        // SAFETY: the handles were created from `vk` by `Surface::new`, the
        // device is idle (`release_swapchain`) and nothing else refers to them.
        unsafe {
            if self.image_avail_sem != 0 {
                (vk.lib.vkDestroySemaphore)(vk.device, self.image_avail_sem, std::ptr::null());
            }
            if self.render_done_sem != 0 {
                (vk.lib.vkDestroySemaphore)(vk.device, self.render_done_sem, std::ptr::null());
            }
            if self.fence != 0 {
                (vk.lib.vkDestroyFence)(vk.device, self.fence, std::ptr::null());
            }
            if self.surface_khr != 0 {
                (vk.lib.vkDestroySurfaceKHR)(vk.instance, self.surface_khr, std::ptr::null());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framebuffer_extent_tracks_register_and_forget() {
        // Handles unlikely to collide with a real framebuffer in other tests.
        let fb = 0xDEAD_0001;
        assert_eq!(framebuffer_extent(fb), None);
        register_framebuffer(fb, 968, 620);
        assert_eq!(framebuffer_extent(fb), Some((968, 620)));
        register_framebuffer(fb, 1200, 800);
        assert_eq!(framebuffer_extent(fb), Some((1200, 800)));
        forget_framebuffer(fb);
        assert_eq!(framebuffer_extent(fb), None);
    }

    fn caps(current: (u32, u32), min: (u32, u32), max: (u32, u32)) -> VkSurfaceCapabilitiesKHR {
        VkSurfaceCapabilitiesKHR {
            current_extent: VkExtent2D { width: current.0, height: current.1 },
            min_image_extent: VkExtent2D { width: min.0, height: min.1 },
            max_image_extent: VkExtent2D { width: max.0, height: max.1 },
            ..VkSurfaceCapabilitiesKHR::default()
        }
    }

    fn extent(c: &VkSurfaceCapabilitiesKHR, w: u32, h: u32) -> Option<(u32, u32)> {
        swapchain_extent(c, w, h).map(|e| (e.width, e.height))
    }

    #[test]
    fn swapchain_extent_follows_a_fixed_surface_size() {
        // X11: the surface dictates the extent; a stale request must not be used.
        let c = caps((1200, 800), (1200, 800), (1200, 800));
        assert_eq!(extent(&c, 968, 620), Some((1200, 800)));
    }

    #[test]
    fn swapchain_extent_clamps_a_free_surface_size() {
        // Wayland: 0xFFFFFFFF means "the swapchain decides", within the limits.
        let c = caps((u32::MAX, u32::MAX), (1, 1), (4096, 4096));
        assert_eq!(extent(&c, 968, 620), Some((968, 620)));
        assert_eq!(extent(&c, 9000, 0), Some((4096, 1)));
    }

    #[test]
    fn swapchain_extent_is_none_for_a_zero_area_surface() {
        // Minimised window, or a failed capabilities query (all zero).
        assert_eq!(extent(&caps((0, 0), (0, 0), (0, 0)), 968, 620), None);
        assert_eq!(extent(&VkSurfaceCapabilitiesKHR::default(), 968, 620), None);
    }
}
