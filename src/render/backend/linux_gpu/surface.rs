//! Wayland / X11 `VkSurfaceKHR` and `VkSwapchainKHR` (`VK_PRESENT_MODE_FIFO_KHR`, `minImageCount = 2`).

use std::ffi::c_void;
use std::sync::Arc;

use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use winit::window::Window;

use super::device::{shared_pipelines, Device, Queue, VulkanCore};
use super::ffi::*;
use crate::render::backend::{SurfaceConfig, TextureFormat};

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
    pub width: u32,
    pub height: u32,
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
                if let (Ok(wh), Ok(dh)) = (window.window_handle(), window.display_handle()) {
                    match (wh.as_raw(), dh.as_raw()) {
                        (RawWindowHandle::Wayland(w), RawDisplayHandle::Wayland(d)) => {
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
                                    display: d.display.as_ptr(),
                                    surface: w.surface.as_ptr(),
                                };
                                create_wl(vk.instance, &info as *const _ as *const c_void, std::ptr::null(), &mut surface_khr);
                            }
                        }
                        (RawWindowHandle::Xlib(w), RawDisplayHandle::Xlib(d)) => {
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
                                    dpy: d.display.map_or(std::ptr::null_mut(), |p| p.as_ptr()),
                                    window: w.window,
                                };
                                create_xlib(vk.instance, &info as *const _ as *const c_void, std::ptr::null(), &mut surface_khr);
                            }
                        }
                        (RawWindowHandle::Xcb(w), RawDisplayHandle::Xcb(d)) => {
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
                                    connection: d.connection.map_or(std::ptr::null_mut(), |p| p.as_ptr()),
                                    window: w.window.get(),
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
                (vk.lib.vkDestroyFramebuffer)(vk.device, fb, std::ptr::null());
            }
            self.framebuffers.clear();
            for &v in &self.views {
                (vk.lib.vkDestroyImageView)(vk.device, v, std::ptr::null());
            }
            self.views.clear();

            let mut caps = VkSurfaceCapabilitiesKHR::default();
            (vk.lib.vkGetPhysicalDeviceSurfaceCapabilitiesKHR)(vk.phys, self.surface_khr, &mut caps);

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
                image_extent: VkExtent2D { width: self.width, height: self.height },
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
            if (vk.lib.vkCreateSwapchainKHR)(vk.device, &sc_info as *const _ as *const c_void, std::ptr::null(), &mut new_sc) == VK_SUCCESS {
                self.swapchain = new_sc;
            }
            if old_sc != 0 {
                (vk.lib.vkDestroySwapchainKHR)(vk.device, old_sc, std::ptr::null());
            }
            if self.swapchain == 0 {
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
                width: self.width,
                height: self.height,
            })
        }
    }

    pub fn shrink_drawables(&mut self, _device: &Device, _queue: &Queue) {
        let Some(vk) = &self.vk else { return };
        unsafe {
            (vk.lib.vkDeviceWaitIdle)(vk.device);
            for &fb in &self.framebuffers {
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
