//! Vulkan instance, physical/logical device, SPIR-V shader modules, and cached graphics pipelines.

use std::ffi::{c_char, c_void, CStr};
use std::sync::Arc;

use super::ffi::*;
use crate::render::backend::TextureFormat;

const SHADER_SPV: &[u8] = include_bytes!("../../shaders/shader.spv");
const IMAGE_SPV: &[u8] = include_bytes!("../../shaders/image.spv");

pub struct VulkanCore {
    pub(super) lib: VkLib,
    pub(super) instance: VkInstance,
    pub(super) phys: VkPhysicalDevice,
    pub(super) device: VkDevice,
    pub(super) queue: VkQueue,
    pub(super) mem_props: VkPhysicalDeviceMemoryProperties,
    #[allow(dead_code)]
    pub(super) cmd_pool: VkCommandPool,
    pub(super) upload_cmd: VkCommandBuffer,
    #[allow(dead_code)]
    pub(super) desc_pool: VkDescriptorPool,
    /// One set per sampled texture, for the image pipeline; sets are freed
    /// with their texture, so the pool allows it.
    pub(super) image_desc_pool: VkDescriptorPool,
    /// Descriptor pools are not thread safe; guards `image_desc_pool`.
    pub(super) image_pool_lock: parking_lot::Mutex<()>,
    /// Shared upload staging buffer. Its lock is held for a whole upload, so
    /// it also serialises every use of `upload_cmd` outside a frame.
    pub(super) staging: parking_lot::Mutex<super::resources::Staging>,
    pub(super) shader_mod: VkShaderModule,
    pub(super) image_mod: VkShaderModule,
    #[allow(dead_code)]
    pub(super) cells_dset_layout: VkDescriptorSetLayout,
    pub(super) cells_pipe_layout: VkPipelineLayout,
    #[allow(dead_code)]
    pub(super) image_dset_layout: VkDescriptorSetLayout,
    pub(super) image_pipe_layout: VkPipelineLayout,
    pub(super) nearest_sampler: VkSampler,
    pub(super) linear_sampler: VkSampler,
    pub(super) cells_dsets: [VkDescriptorSet; 2],
    pub(super) cells_ubufs: [(VkBuffer, *mut u8); 2],
    pub(super) image_ubuf: (VkBuffer, *mut u8),
}

unsafe impl Send for VulkanCore {}
unsafe impl Sync for VulkanCore {}

#[derive(Clone)]
pub struct Device {
    pub(super) vk: Option<Arc<VulkanCore>>,
}

#[derive(Clone)]
pub struct Queue {
    pub(super) vk: Option<Arc<VulkanCore>>,
}

#[derive(Clone)]
pub struct RenderPipelines {
    pub cells: ObjPtr,
    pub corner: ObjPtr,
    pub image: ObjPtr,
    pub render_pass_clear: u64,
    pub render_pass_load: u64,
}

static SHARED_VK: parking_lot::Mutex<Option<(Arc<Device>, Arc<Queue>)>> = parking_lot::const_mutex(None);
pub(super) static SHARED_PIPES: parking_lot::Mutex<Option<(TextureFormat, RenderPipelines)>> =
    parking_lot::const_mutex(None);

/// Sampled textures that can own an image descriptor set at once (glyph
/// atlases plus inline images; a terminal keeps at most 1024 images).
const IMAGE_DESCRIPTOR_SETS: u32 = 2048;

/// Window systems a surface can be created for, in no particular order.
const WINDOW_SYSTEM_EXTENSIONS: [&CStr; 3] =
    [c"VK_KHR_wayland_surface", c"VK_KHR_xlib_surface", c"VK_KHR_xcb_surface"];

/// `VK_KHR_surface` plus each window-system extension the loader offers.
/// Asking for one it lacks fails the whole instance, so only those are
/// requested. `None` when no window could be presented to at all.
fn instance_extensions(available: &[&CStr]) -> Option<Vec<&'static CStr>> {
    let surface = c"VK_KHR_surface";
    if !available.contains(&surface) {
        return None;
    }
    let mut extensions = vec![surface];
    extensions.extend(WINDOW_SYSTEM_EXTENSIONS.into_iter().filter(|e| available.contains(e)));
    (extensions.len() > 1).then_some(extensions)
}

/// Index of the first queue family that can run graphics commands.
fn graphics_family(families: &[VkQueueFamilyProperties]) -> Option<u32> {
    families
        .iter()
        .position(|f| f.queue_count > 0 && f.queue_flags & VK_QUEUE_GRAPHICS_BIT != 0)
        .map(|i| i as u32)
}

impl VulkanCore {
    #[allow(non_snake_case)]
    // `load_fn!` transmutes to the fn-pointer type of the struct field it initialises; spelling out
    // each of those signatures a second time at the call site would only duplicate `VulkanFns`.
    #[allow(clippy::missing_transmute_annotations)]
    unsafe fn try_init() -> Option<Arc<Self>> {
        unsafe {
            let handle = libc::dlopen(c"libvulkan.so.1".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
            if handle.is_null() {
                return None;
            }
            let gipa_ptr = libc::dlsym(handle, c"vkGetInstanceProcAddr".as_ptr());
            if gipa_ptr.is_null() {
                return None;
            }
            let vkGetInstanceProcAddr: unsafe extern "C" fn(VkInstance, *const c_char) -> *const c_void =
                std::mem::transmute(gipa_ptr);

            let vkCreateInstance: unsafe extern "C" fn(*const c_void, *const c_void, *mut VkInstance) -> i32 =
                std::mem::transmute(vkGetInstanceProcAddr(std::ptr::null_mut(), c"vkCreateInstance".as_ptr()));

            #[repr(C)]
            struct AppInfo {
                s_type: u32,
                p_next: *const c_void,
                p_app_name: *const c_char,
                app_ver: u32,
                p_eng_name: *const c_char,
                eng_ver: u32,
                api_ver: u32,
            }
            #[repr(C)]
            struct InstInfo {
                s_type: u32,
                p_next: *const c_void,
                flags: u32,
                p_app_info: *const AppInfo,
                layer_count: u32,
                pp_layers: *const *const c_char,
                ext_count: u32,
                pp_exts: *const *const c_char,
            }
            let app_info = AppInfo {
                s_type: VK_STRUCTURE_TYPE_APPLICATION_INFO,
                p_next: std::ptr::null(),
                p_app_name: c"litty".as_ptr(),
                app_ver: 1,
                p_eng_name: c"litty".as_ptr(),
                eng_ver: 1,
                api_ver: 1 << 22,
            };
            let vkEnumerateInstanceExtensionProperties: unsafe extern "C" fn(
                *const c_char,
                *mut u32,
                *mut VkExtensionProperties,
            ) -> i32 = {
                let p = vkGetInstanceProcAddr(std::ptr::null_mut(), c"vkEnumerateInstanceExtensionProperties".as_ptr());
                if p.is_null() {
                    return None;
                }
                std::mem::transmute(p)
            };
            let mut ext_count = 0u32;
            vkEnumerateInstanceExtensionProperties(std::ptr::null(), &mut ext_count, std::ptr::null_mut());
            let mut ext_props =
                vec![VkExtensionProperties { extension_name: [0; 256], spec_version: 0 }; ext_count as usize];
            vkEnumerateInstanceExtensionProperties(std::ptr::null(), &mut ext_count, ext_props.as_mut_ptr());
            ext_props.truncate(ext_count as usize);
            // SAFETY: the loader NUL-terminates every name within its 256 bytes.
            let available: Vec<&CStr> =
                ext_props.iter().map(|p| CStr::from_ptr(p.extension_name.as_ptr())).collect();
            let Some(extensions) = instance_extensions(&available) else {
                eprintln!("litty: the Vulkan loader offers no window-system surface extension");
                return None;
            };
            let exts_ws: Vec<*const c_char> = extensions.iter().map(|e| e.as_ptr()).collect();
            let mut instance: VkInstance = std::ptr::null_mut();
            let inst_info = InstInfo {
                s_type: VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO,
                p_next: std::ptr::null(),
                flags: 0,
                p_app_info: &app_info,
                layer_count: 0,
                pp_layers: std::ptr::null(),
                ext_count: exts_ws.len() as u32,
                pp_exts: exts_ws.as_ptr(),
            };
            if vkCreateInstance(&inst_info as *const _ as *const c_void, std::ptr::null(), &mut instance) != VK_SUCCESS {
                eprintln!("litty: could not create a Vulkan instance");
                return None;
            }

            macro_rules! load_fn {
                ($name:ident) => {{
                    let p = vkGetInstanceProcAddr(
                        instance,
                        CStr::from_bytes_with_nul_unchecked(concat!(stringify!($name), "\0").as_bytes()).as_ptr(),
                    );
                    if p.is_null() {
                        return None;
                    }
                    std::mem::transmute(p)
                }};
            }
            macro_rules! load_opt_fn {
                ($name:ident) => {{
                    let p = vkGetInstanceProcAddr(
                        instance,
                        CStr::from_bytes_with_nul_unchecked(concat!(stringify!($name), "\0").as_bytes()).as_ptr(),
                    );
                    if p.is_null() {
                        None
                    } else {
                        Some(std::mem::transmute(p))
                    }
                }};
            }

            let vkEnumeratePhysicalDevices: unsafe extern "C" fn(VkInstance, *mut u32, *mut VkPhysicalDevice) -> i32 =
                load_fn!(vkEnumeratePhysicalDevices);
            let mut pd_count = 0u32;
            if vkEnumeratePhysicalDevices(instance, &mut pd_count, std::ptr::null_mut()) != VK_SUCCESS || pd_count == 0 {
                return None;
            }
            let mut pds = vec![std::ptr::null_mut(); pd_count as usize];
            vkEnumeratePhysicalDevices(instance, &mut pd_count, pds.as_mut_ptr());
            let vkGetPhysicalDeviceQueueFamilyProperties: unsafe extern "C" fn(
                VkPhysicalDevice,
                *mut u32,
                *mut VkQueueFamilyProperties,
            ) = load_fn!(vkGetPhysicalDeviceQueueFamilyProperties);
            let mut chosen = None;
            for &candidate in &pds {
                let mut family_count = 0u32;
                vkGetPhysicalDeviceQueueFamilyProperties(candidate, &mut family_count, std::ptr::null_mut());
                let mut families = vec![VkQueueFamilyProperties::default(); family_count as usize];
                vkGetPhysicalDeviceQueueFamilyProperties(candidate, &mut family_count, families.as_mut_ptr());
                families.truncate(family_count as usize);
                if let Some(family) = graphics_family(&families) {
                    chosen = Some((candidate, family));
                    break;
                }
            }
            let Some((phys, queue_family)) = chosen else {
                eprintln!("litty: no Vulkan device with a graphics queue");
                return None;
            };

            let vkGetPhysicalDeviceMemoryProperties: unsafe extern "C" fn(
                VkPhysicalDevice,
                *mut VkPhysicalDeviceMemoryProperties,
            ) = load_fn!(vkGetPhysicalDeviceMemoryProperties);
            let mut mem_props = VkPhysicalDeviceMemoryProperties::default();
            vkGetPhysicalDeviceMemoryProperties(phys, &mut mem_props);

            let vkCreateDevice: unsafe extern "C" fn(
                VkPhysicalDevice,
                *const c_void,
                *const c_void,
                *mut VkDevice,
            ) -> i32 = load_fn!(vkCreateDevice);
            let prio = 1.0f32;
            #[repr(C)]
            struct QueueInfo {
                s_type: u32,
                p_next: *const c_void,
                flags: u32,
                queue_family_index: u32,
                queue_count: u32,
                p_priorities: *const f32,
            }
            #[repr(C)]
            struct DevInfo {
                s_type: u32,
                p_next: *const c_void,
                flags: u32,
                queue_info_count: u32,
                p_queue_infos: *const QueueInfo,
                layer_count: u32,
                pp_layers: *const *const c_char,
                ext_count: u32,
                pp_exts: *const *const c_char,
                p_features: *const c_void,
            }
            let q_info = QueueInfo {
                s_type: VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO,
                p_next: std::ptr::null(),
                flags: 0,
                queue_family_index: queue_family,
                queue_count: 1,
                p_priorities: &prio,
            };
            let dev_exts = [c"VK_KHR_swapchain".as_ptr()];
            let mut dev_info = DevInfo {
                s_type: VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO,
                p_next: std::ptr::null(),
                flags: 0,
                queue_info_count: 1,
                p_queue_infos: &q_info,
                layer_count: 0,
                pp_layers: std::ptr::null(),
                ext_count: 1,
                pp_exts: dev_exts.as_ptr(),
                p_features: std::ptr::null(),
            };
            let mut device: VkDevice = std::ptr::null_mut();
            if vkCreateDevice(phys, &dev_info as *const _ as *const c_void, std::ptr::null(), &mut device) != VK_SUCCESS {
                dev_info.ext_count = 0;
                dev_info.pp_exts = std::ptr::null();
                if vkCreateDevice(phys, &dev_info as *const _ as *const c_void, std::ptr::null(), &mut device) != VK_SUCCESS {
                    return None;
                }
            }

            let lib = VkLib {
                _handle: handle,
                vkGetInstanceProcAddr,
                vkCreateInstance,
                vkEnumeratePhysicalDevices,
                vkGetPhysicalDeviceMemoryProperties,
                vkCreateDevice,
                vkGetDeviceQueue: load_fn!(vkGetDeviceQueue),
                vkCreateBuffer: load_fn!(vkCreateBuffer),
                vkDestroyBuffer: load_fn!(vkDestroyBuffer),
                vkGetBufferMemoryRequirements: load_fn!(vkGetBufferMemoryRequirements),
                vkAllocateMemory: load_fn!(vkAllocateMemory),
                vkFreeMemory: load_fn!(vkFreeMemory),
                vkBindBufferMemory: load_fn!(vkBindBufferMemory),
                vkMapMemory: load_fn!(vkMapMemory),
                vkCreateImage: load_fn!(vkCreateImage),
                vkDestroyImage: load_fn!(vkDestroyImage),
                vkGetImageMemoryRequirements: load_fn!(vkGetImageMemoryRequirements),
                vkBindImageMemory: load_fn!(vkBindImageMemory),
                vkCreateImageView: load_fn!(vkCreateImageView),
                vkDestroyImageView: load_fn!(vkDestroyImageView),
                vkCreateSampler: load_fn!(vkCreateSampler),
                vkCreateShaderModule: load_fn!(vkCreateShaderModule),
                vkCreateDescriptorSetLayout: load_fn!(vkCreateDescriptorSetLayout),
                vkCreatePipelineLayout: load_fn!(vkCreatePipelineLayout),
                vkCreateRenderPass: load_fn!(vkCreateRenderPass),
                vkCreateGraphicsPipelines: load_fn!(vkCreateGraphicsPipelines),
                vkCreateDescriptorPool: load_fn!(vkCreateDescriptorPool),
                vkAllocateDescriptorSets: load_fn!(vkAllocateDescriptorSets),
                vkUpdateDescriptorSets: load_fn!(vkUpdateDescriptorSets),
                vkFreeDescriptorSets: load_fn!(vkFreeDescriptorSets),
                vkCreateFramebuffer: load_fn!(vkCreateFramebuffer),
                vkDestroyFramebuffer: load_fn!(vkDestroyFramebuffer),
                vkCreateCommandPool: load_fn!(vkCreateCommandPool),
                vkAllocateCommandBuffers: load_fn!(vkAllocateCommandBuffers),
                vkResetCommandBuffer: load_fn!(vkResetCommandBuffer),
                vkBeginCommandBuffer: load_fn!(vkBeginCommandBuffer),
                vkEndCommandBuffer: load_fn!(vkEndCommandBuffer),
                vkCmdBeginRenderPass: load_fn!(vkCmdBeginRenderPass),
                vkCmdEndRenderPass: load_fn!(vkCmdEndRenderPass),
                vkCmdBindPipeline: load_fn!(vkCmdBindPipeline),
                vkCmdBindDescriptorSets: load_fn!(vkCmdBindDescriptorSets),
                vkCmdBindVertexBuffers: load_fn!(vkCmdBindVertexBuffers),
                vkCmdSetViewport: load_fn!(vkCmdSetViewport),
                vkCmdSetScissor: load_fn!(vkCmdSetScissor),
                vkCmdDraw: load_fn!(vkCmdDraw),
                vkCmdPipelineBarrier: load_fn!(vkCmdPipelineBarrier),
                vkCmdCopyBufferToImage: load_fn!(vkCmdCopyBufferToImage),
                vkQueueSubmit: load_fn!(vkQueueSubmit),
                vkQueueWaitIdle: load_fn!(vkQueueWaitIdle),
                vkDeviceWaitIdle: load_fn!(vkDeviceWaitIdle),
                vkCreateSemaphore: load_fn!(vkCreateSemaphore),
                vkCreateFence: load_fn!(vkCreateFence),
                vkWaitForFences: load_fn!(vkWaitForFences),
                vkResetFences: load_fn!(vkResetFences),
                vkDestroySemaphore: load_fn!(vkDestroySemaphore),
                vkDestroyFence: load_fn!(vkDestroyFence),
                vkCreateWaylandSurfaceKHR: load_opt_fn!(vkCreateWaylandSurfaceKHR),
                vkCreateXlibSurfaceKHR: load_opt_fn!(vkCreateXlibSurfaceKHR),
                vkCreateXcbSurfaceKHR: load_opt_fn!(vkCreateXcbSurfaceKHR),
                vkGetPhysicalDeviceSurfaceCapabilitiesKHR: load_fn!(vkGetPhysicalDeviceSurfaceCapabilitiesKHR),
                vkCreateSwapchainKHR: load_fn!(vkCreateSwapchainKHR),
                vkDestroySwapchainKHR: load_fn!(vkDestroySwapchainKHR),
                vkGetSwapchainImagesKHR: load_fn!(vkGetSwapchainImagesKHR),
                vkAcquireNextImageKHR: load_fn!(vkAcquireNextImageKHR),
                vkQueuePresentKHR: load_fn!(vkQueuePresentKHR),
                vkDestroySurfaceKHR: load_fn!(vkDestroySurfaceKHR),
            };

            let mut queue: VkQueue = std::ptr::null_mut();
            (lib.vkGetDeviceQueue)(device, queue_family, 0, &mut queue);

            #[repr(C)]
            struct PoolInfo {
                s_type: u32,
                p_next: *const c_void,
                flags: u32,
                queue_family_index: u32,
            }
            let pool_info = PoolInfo {
                s_type: VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO,
                p_next: std::ptr::null(),
                flags: 2,
                queue_family_index: queue_family,
            };
            let mut cmd_pool = 0u64;
            (lib.vkCreateCommandPool)(device, &pool_info as *const _ as *const c_void, std::ptr::null(), &mut cmd_pool);

            #[repr(C)]
            struct CmdAlloc {
                s_type: u32,
                p_next: *const c_void,
                command_pool: VkCommandPool,
                level: u32,
                count: u32,
            }
            let cmd_alloc = CmdAlloc {
                s_type: VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO,
                p_next: std::ptr::null(),
                command_pool: cmd_pool,
                level: 0,
                count: 1,
            };
            let mut upload_cmd: VkCommandBuffer = std::ptr::null_mut();
            (lib.vkAllocateCommandBuffers)(device, &cmd_alloc as *const _ as *const c_void, &mut upload_cmd);

            let shader_mod = create_shader_module(&lib, device, SHADER_SPV);
            let image_mod = create_shader_module(&lib, device, IMAGE_SPV);
            let nearest_sampler = create_vk_sampler(&lib, device, VK_FILTER_NEAREST);
            let linear_sampler = create_vk_sampler(&lib, device, VK_FILTER_LINEAR);

            #[repr(C)]
            #[derive(Copy, Clone)]
            struct Binding {
                binding: u32,
                descriptor_type: u32,
                descriptor_count: u32,
                stage_flags: u32,
                p_immutable_samplers: *const VkSampler,
            }
            #[repr(C)]
            struct DsetLayoutInfo {
                s_type: u32,
                p_next: *const c_void,
                flags: u32,
                binding_count: u32,
                p_bindings: *const Binding,
            }
            let cells_bindings = [
                Binding { binding: 0, descriptor_type: VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER, descriptor_count: 1, stage_flags: VK_SHADER_STAGE_ALL_GRAPHICS, p_immutable_samplers: std::ptr::null() },
                Binding { binding: 1, descriptor_type: VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE, descriptor_count: 1, stage_flags: VK_SHADER_STAGE_FRAGMENT_BIT, p_immutable_samplers: std::ptr::null() },
                Binding { binding: 2, descriptor_type: VK_DESCRIPTOR_TYPE_SAMPLER, descriptor_count: 1, stage_flags: VK_SHADER_STAGE_FRAGMENT_BIT, p_immutable_samplers: std::ptr::null() },
                Binding { binding: 3, descriptor_type: VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE, descriptor_count: 1, stage_flags: VK_SHADER_STAGE_FRAGMENT_BIT, p_immutable_samplers: std::ptr::null() },
                Binding { binding: 4, descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER, descriptor_count: 1, stage_flags: VK_SHADER_STAGE_VERTEX_BIT, p_immutable_samplers: std::ptr::null() },
            ];
            let cells_dl_info = DsetLayoutInfo {
                s_type: VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO,
                p_next: std::ptr::null(),
                flags: 0,
                binding_count: cells_bindings.len() as u32,
                p_bindings: cells_bindings.as_ptr(),
            };
            let mut cells_dset_layout = 0u64;
            (lib.vkCreateDescriptorSetLayout)(device, &cells_dl_info as *const _ as *const c_void, std::ptr::null(), &mut cells_dset_layout);

            let image_bindings = [
                Binding { binding: 0, descriptor_type: VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER, descriptor_count: 1, stage_flags: VK_SHADER_STAGE_ALL_GRAPHICS, p_immutable_samplers: std::ptr::null() },
                Binding { binding: 1, descriptor_type: VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE, descriptor_count: 1, stage_flags: VK_SHADER_STAGE_FRAGMENT_BIT, p_immutable_samplers: std::ptr::null() },
                Binding { binding: 2, descriptor_type: VK_DESCRIPTOR_TYPE_SAMPLER, descriptor_count: 1, stage_flags: VK_SHADER_STAGE_FRAGMENT_BIT, p_immutable_samplers: std::ptr::null() },
            ];
            let image_dl_info = DsetLayoutInfo {
                s_type: VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO,
                p_next: std::ptr::null(),
                flags: 0,
                binding_count: image_bindings.len() as u32,
                p_bindings: image_bindings.as_ptr(),
            };
            let mut image_dset_layout = 0u64;
            (lib.vkCreateDescriptorSetLayout)(device, &image_dl_info as *const _ as *const c_void, std::ptr::null(), &mut image_dset_layout);

            #[repr(C)]
            struct PipeLayoutInfo {
                s_type: u32,
                p_next: *const c_void,
                flags: u32,
                set_layout_count: u32,
                p_set_layouts: *const VkDescriptorSetLayout,
                push_constant_range_count: u32,
                p_push_constant_ranges: *const c_void,
            }
            let mut cells_pipe_layout = 0u64;
            let c_pl_info = PipeLayoutInfo {
                s_type: VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO,
                p_next: std::ptr::null(),
                flags: 0,
                set_layout_count: 1,
                p_set_layouts: &cells_dset_layout,
                push_constant_range_count: 0,
                p_push_constant_ranges: std::ptr::null(),
            };
            (lib.vkCreatePipelineLayout)(device, &c_pl_info as *const _ as *const c_void, std::ptr::null(), &mut cells_pipe_layout);

            let mut image_pipe_layout = 0u64;
            let i_pl_info = PipeLayoutInfo {
                s_type: VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO,
                p_next: std::ptr::null(),
                flags: 0,
                set_layout_count: 1,
                p_set_layouts: &image_dset_layout,
                push_constant_range_count: 0,
                p_push_constant_ranges: std::ptr::null(),
            };
            (lib.vkCreatePipelineLayout)(device, &i_pl_info as *const _ as *const c_void, std::ptr::null(), &mut image_pipe_layout);

            #[repr(C)]
            struct PoolSize {
                ty: u32,
                count: u32,
            }
            #[repr(C)]
            struct DescPoolInfo {
                s_type: u32,
                p_next: *const c_void,
                flags: u32,
                max_sets: u32,
                pool_size_count: u32,
                p_pool_sizes: *const PoolSize,
            }
            let pool_sizes = [
                PoolSize { ty: VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER, count: 64 },
                PoolSize { ty: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER, count: 64 },
                PoolSize { ty: VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE, count: 128 },
                PoolSize { ty: VK_DESCRIPTOR_TYPE_SAMPLER, count: 64 },
            ];
            let dp_info = DescPoolInfo {
                s_type: VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO,
                p_next: std::ptr::null(),
                flags: 0,
                max_sets: 64,
                pool_size_count: pool_sizes.len() as u32,
                p_pool_sizes: pool_sizes.as_ptr(),
            };
            let mut desc_pool = 0u64;
            (lib.vkCreateDescriptorPool)(device, &dp_info as *const _ as *const c_void, std::ptr::null(), &mut desc_pool);
            let image_pool_sizes = [
                PoolSize { ty: VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER, count: IMAGE_DESCRIPTOR_SETS },
                PoolSize { ty: VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE, count: IMAGE_DESCRIPTOR_SETS },
                PoolSize { ty: VK_DESCRIPTOR_TYPE_SAMPLER, count: IMAGE_DESCRIPTOR_SETS },
            ];
            let image_dp_info = DescPoolInfo {
                s_type: VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO,
                p_next: std::ptr::null(),
                flags: VK_DESCRIPTOR_POOL_CREATE_FREE_DESCRIPTOR_SET_BIT,
                max_sets: IMAGE_DESCRIPTOR_SETS,
                pool_size_count: image_pool_sizes.len() as u32,
                p_pool_sizes: image_pool_sizes.as_ptr(),
            };
            let mut image_desc_pool = 0u64;
            (lib.vkCreateDescriptorPool)(device, &image_dp_info as *const _ as *const c_void, std::ptr::null(), &mut image_desc_pool);

            #[repr(C)]
            struct DsetAlloc {
                s_type: u32,
                p_next: *const c_void,
                descriptor_pool: VkDescriptorPool,
                descriptor_set_count: u32,
                p_set_layouts: *const VkDescriptorSetLayout,
            }
            let layouts = [cells_dset_layout, cells_dset_layout];
            let mut cells_dsets = [0u64; 2];
            let ds_alloc = DsetAlloc {
                s_type: VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO,
                p_next: std::ptr::null(),
                descriptor_pool: desc_pool,
                descriptor_set_count: 2,
                p_set_layouts: layouts.as_ptr(),
            };
            (lib.vkAllocateDescriptorSets)(device, &ds_alloc as *const _ as *const c_void, cells_dsets.as_mut_ptr());

            let (ubuf0, _, uptr0) = super::resources::alloc_host_buffer(&lib, device, &mem_props, 256, VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT);
            let (ubuf1, _, uptr1) = super::resources::alloc_host_buffer(&lib, device, &mem_props, 256, VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT);
            let (iubuf, _, iuptr) = super::resources::alloc_host_buffer(&lib, device, &mem_props, 64, VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT);

            Some(Arc::new(Self {
                lib,
                instance,
                phys,
                device,
                queue,
                mem_props,
                cmd_pool,
                upload_cmd,
                desc_pool,
                image_desc_pool,
                image_pool_lock: parking_lot::Mutex::new(()),
                staging: parking_lot::Mutex::new(super::resources::Staging::empty()),
                shader_mod,
                image_mod,
                cells_dset_layout,
                cells_pipe_layout,
                image_dset_layout,
                image_pipe_layout,
                nearest_sampler,
                linear_sampler,
                cells_dsets,
                cells_ubufs: [(ubuf0, uptr0), (ubuf1, uptr1)],
                image_ubuf: (iubuf, iuptr),
            }))
        }
    }
}

unsafe fn create_shader_module(lib: &VkLib, device: VkDevice, spv: &[u8]) -> VkShaderModule {
    unsafe {
        #[repr(C)]
        struct ModInfo {
            s_type: u32,
            p_next: *const c_void,
            flags: u32,
            code_size: usize,
            p_code: *const u32,
        }
        let aligned: Vec<u32> = spv.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect();
        let info = ModInfo {
            s_type: VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            code_size: aligned.len() * 4,
            p_code: aligned.as_ptr(),
        };
        let mut module = 0u64;
        (lib.vkCreateShaderModule)(device, &info as *const _ as *const c_void, std::ptr::null(), &mut module);
        module
    }
}

unsafe fn create_vk_sampler(lib: &VkLib, device: VkDevice, filter: u32) -> VkSampler {
    unsafe {
        #[repr(C)]
        struct SamplerInfo {
            s_type: u32,
            p_next: *const c_void,
            flags: u32,
            mag_filter: u32,
            min_filter: u32,
            mipmap_mode: u32,
            address_mode_u: u32,
            address_mode_v: u32,
            address_mode_w: u32,
            mip_lod_bias: f32,
            anisotropy_enable: VkBool32,
            max_anisotropy: f32,
            compare_enable: VkBool32,
            compare_op: u32,
            min_lod: f32,
            max_lod: f32,
            border_color: u32,
            unnormalized_coordinates: VkBool32,
        }
        let info = SamplerInfo {
            s_type: VK_STRUCTURE_TYPE_SAMPLER_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            mag_filter: filter,
            min_filter: filter,
            mipmap_mode: 0,
            address_mode_u: VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE,
            address_mode_v: VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE,
            address_mode_w: VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE,
            mip_lod_bias: 0.0,
            anisotropy_enable: 0,
            max_anisotropy: 1.0,
            compare_enable: 0,
            compare_op: 0,
            min_lod: 0.0,
            max_lod: 0.0,
            border_color: 0,
            unnormalized_coordinates: 0,
        };
        let mut sampler = 0u64;
        (lib.vkCreateSampler)(device, &info as *const _ as *const c_void, std::ptr::null(), &mut sampler);
        sampler
    }
}

unsafe fn create_render_pass(lib: &VkLib, device: VkDevice, format: TextureFormat, clear: bool) -> VkRenderPass {
    unsafe {
        #[repr(C)]
        struct AttDesc {
            flags: u32,
            format: u32,
            samples: u32,
            load_op: u32,
            store_op: u32,
            stencil_load_op: u32,
            stencil_store_op: u32,
            initial_layout: u32,
            final_layout: u32,
        }
        #[repr(C)]
        struct AttRef {
            attachment: u32,
            layout: u32,
        }
        #[repr(C)]
        struct SubpassDesc {
            flags: u32,
            pipeline_bind_point: u32,
            input_attachment_count: u32,
            p_input_attachments: *const AttRef,
            color_attachment_count: u32,
            p_color_attachments: *const AttRef,
            p_resolve_attachments: *const AttRef,
            p_depth_stencil_attachment: *const AttRef,
            preserve_attachment_count: u32,
            p_preserve_attachments: *const u32,
        }
        #[repr(C)]
        struct RpInfo {
            s_type: u32,
            p_next: *const c_void,
            flags: u32,
            attachment_count: u32,
            p_attachments: *const AttDesc,
            subpass_count: u32,
            p_subpasses: *const SubpassDesc,
            dependency_count: u32,
            p_dependencies: *const c_void,
        }
        let att = AttDesc {
            flags: 0,
            format: vk_format(format),
            samples: 1,
            load_op: if clear { VK_ATTACHMENT_LOAD_OP_CLEAR } else { VK_ATTACHMENT_LOAD_OP_LOAD },
            store_op: VK_ATTACHMENT_STORE_OP_STORE,
            stencil_load_op: VK_ATTACHMENT_LOAD_OP_DONT_CARE,
            stencil_store_op: 1,
            initial_layout: if clear { VK_IMAGE_LAYOUT_UNDEFINED } else { VK_IMAGE_LAYOUT_PRESENT_SRC_KHR },
            final_layout: VK_IMAGE_LAYOUT_PRESENT_SRC_KHR,
        };
        let color_ref = AttRef { attachment: 0, layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL };
        let subpass = SubpassDesc {
            flags: 0,
            pipeline_bind_point: 0,
            input_attachment_count: 0,
            p_input_attachments: std::ptr::null(),
            color_attachment_count: 1,
            p_color_attachments: &color_ref,
            p_resolve_attachments: std::ptr::null(),
            p_depth_stencil_attachment: std::ptr::null(),
            preserve_attachment_count: 0,
            p_preserve_attachments: std::ptr::null(),
        };
        let rp_info = RpInfo {
            s_type: VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            attachment_count: 1,
            p_attachments: &att,
            subpass_count: 1,
            p_subpasses: &subpass,
            dependency_count: 0,
            p_dependencies: std::ptr::null(),
        };
        let mut rp = 0u64;
        (lib.vkCreateRenderPass)(device, &rp_info as *const _ as *const c_void, std::ptr::null(), &mut rp);
        rp
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn create_graphics_pipeline(
    lib: &VkLib,
    device: VkDevice,
    render_pass: VkRenderPass,
    pipe_layout: VkPipelineLayout,
    module: VkShaderModule,
    vs_entry: &CStr,
    fs_entry: &CStr,
    vertex_stride: u32,
    attrs: &[(u32, u32, u32)],
    corner_fade: bool,
) -> VkPipeline {
    unsafe {
        #[repr(C)]
        struct StageInfo {
            s_type: u32,
            p_next: *const c_void,
            flags: u32,
            stage: u32,
            module: VkShaderModule,
            p_name: *const c_char,
            p_spec_info: *const c_void,
        }
        let stages = [
            StageInfo {
                s_type: VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO,
                p_next: std::ptr::null(),
                flags: 0,
                stage: VK_SHADER_STAGE_VERTEX_BIT,
                module,
                p_name: vs_entry.as_ptr(),
                p_spec_info: std::ptr::null(),
            },
            StageInfo {
                s_type: VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO,
                p_next: std::ptr::null(),
                flags: 0,
                stage: VK_SHADER_STAGE_FRAGMENT_BIT,
                module,
                p_name: fs_entry.as_ptr(),
                p_spec_info: std::ptr::null(),
            },
        ];
        #[repr(C)]
        struct VertBinding {
            binding: u32,
            stride: u32,
            input_rate: u32,
        }
        #[repr(C)]
        struct VertAttr {
            location: u32,
            binding: u32,
            format: u32,
            offset: u32,
        }
        #[repr(C)]
        struct VertInput {
            s_type: u32,
            p_next: *const c_void,
            flags: u32,
            binding_count: u32,
            p_bindings: *const VertBinding,
            attr_count: u32,
            p_attrs: *const VertAttr,
        }
        let v_binding = VertBinding {
            binding: 0,
            stride: vertex_stride,
            input_rate: VK_VERTEX_INPUT_RATE_INSTANCE,
        };
        let v_attrs: Vec<VertAttr> = attrs
            .iter()
            .map(|&(loc, fmt, off)| VertAttr { location: loc, binding: 0, format: fmt, offset: off })
            .collect();
        let vert_input = VertInput {
            s_type: VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            binding_count: if vertex_stride > 0 { 1 } else { 0 },
            p_bindings: if vertex_stride > 0 { &v_binding } else { std::ptr::null() },
            attr_count: v_attrs.len() as u32,
            p_attrs: if v_attrs.is_empty() { std::ptr::null() } else { v_attrs.as_ptr() },
        };
        #[repr(C)]
        struct InputAsm {
            s_type: u32,
            p_next: *const c_void,
            flags: u32,
            topology: u32,
            primitive_restart_enable: VkBool32,
        }
        let input_asm = InputAsm {
            s_type: VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            topology: VK_PRIMITIVE_TOPOLOGY_TRIANGLE_STRIP,
            primitive_restart_enable: 0,
        };
        #[repr(C)]
        struct VpState {
            s_type: u32,
            p_next: *const c_void,
            flags: u32,
            viewport_count: u32,
            p_viewports: *const VkViewport,
            scissor_count: u32,
            p_scissors: *const VkRect2D,
        }
        let vp_state = VpState {
            s_type: VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            viewport_count: 1,
            p_viewports: std::ptr::null(),
            scissor_count: 1,
            p_scissors: std::ptr::null(),
        };
        #[repr(C)]
        struct RasterState {
            s_type: u32,
            p_next: *const c_void,
            flags: u32,
            depth_clamp_enable: VkBool32,
            rasterizer_discard_enable: VkBool32,
            polygon_mode: u32,
            cull_mode: u32,
            front_face: u32,
            depth_bias_enable: VkBool32,
            depth_bias_constant_factor: f32,
            depth_bias_clamp: f32,
            depth_bias_slope_factor: f32,
            line_width: f32,
        }
        let raster = RasterState {
            s_type: VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            depth_clamp_enable: 0,
            rasterizer_discard_enable: 0,
            polygon_mode: 0,
            cull_mode: 0,
            front_face: 0,
            depth_bias_enable: 0,
            depth_bias_constant_factor: 0.0,
            depth_bias_clamp: 0.0,
            depth_bias_slope_factor: 0.0,
            line_width: 1.0,
        };
        #[repr(C)]
        struct MsState {
            s_type: u32,
            p_next: *const c_void,
            flags: u32,
            rasterization_samples: u32,
            sample_shading_enable: VkBool32,
            min_sample_shading: f32,
            p_sample_mask: *const u32,
            alpha_to_coverage_enable: VkBool32,
            alpha_to_one_enable: VkBool32,
        }
        let ms = MsState {
            s_type: VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            rasterization_samples: 1,
            sample_shading_enable: 0,
            min_sample_shading: 1.0,
            p_sample_mask: std::ptr::null(),
            alpha_to_coverage_enable: 0,
            alpha_to_one_enable: 0,
        };
        #[repr(C)]
        struct BlendAtt {
            blend_enable: VkBool32,
            src_color_blend_factor: u32,
            dst_color_blend_factor: u32,
            color_blend_op: u32,
            src_alpha_blend_factor: u32,
            dst_alpha_blend_factor: u32,
            alpha_blend_op: u32,
            color_write_mask: u32,
        }
        let blend_att = if corner_fade {
            BlendAtt {
                blend_enable: 1,
                src_color_blend_factor: VK_BLEND_FACTOR_ZERO,
                dst_color_blend_factor: VK_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA,
                color_blend_op: VK_BLEND_OP_ADD,
                src_alpha_blend_factor: VK_BLEND_FACTOR_ZERO,
                dst_alpha_blend_factor: VK_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA,
                alpha_blend_op: VK_BLEND_OP_ADD,
                color_write_mask: 0xF,
            }
        } else {
            BlendAtt {
                blend_enable: 1,
                src_color_blend_factor: VK_BLEND_FACTOR_SRC_ALPHA,
                dst_color_blend_factor: VK_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA,
                color_blend_op: VK_BLEND_OP_ADD,
                src_alpha_blend_factor: VK_BLEND_FACTOR_ONE,
                dst_alpha_blend_factor: VK_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA,
                alpha_blend_op: VK_BLEND_OP_ADD,
                color_write_mask: 0xF,
            }
        };
        #[repr(C)]
        struct BlendState {
            s_type: u32,
            p_next: *const c_void,
            flags: u32,
            logic_op_enable: VkBool32,
            logic_op: u32,
            attachment_count: u32,
            p_attachments: *const BlendAtt,
            blend_constants: [f32; 4],
        }
        let blend = BlendState {
            s_type: VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            logic_op_enable: 0,
            logic_op: 0,
            attachment_count: 1,
            p_attachments: &blend_att,
            blend_constants: [0.0; 4],
        };
        #[repr(C)]
        struct DynState {
            s_type: u32,
            p_next: *const c_void,
            flags: u32,
            dynamic_state_count: u32,
            p_dynamic_states: *const u32,
        }
        let dyn_states = [VK_DYNAMIC_STATE_VIEWPORT, VK_DYNAMIC_STATE_SCISSOR];
        let dyn_info = DynState {
            s_type: VK_STRUCTURE_TYPE_PIPELINE_DYNAMIC_STATE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            dynamic_state_count: 2,
            p_dynamic_states: dyn_states.as_ptr(),
        };
        #[repr(C)]
        struct GfxPipeInfo {
            s_type: u32,
            p_next: *const c_void,
            flags: u32,
            stage_count: u32,
            p_stages: *const StageInfo,
            p_vertex_input_state: *const VertInput,
            p_input_assembly_state: *const InputAsm,
            p_tessellation_state: *const c_void,
            p_viewport_state: *const VpState,
            p_rasterization_state: *const RasterState,
            p_multisample_state: *const MsState,
            p_depth_stencil_state: *const c_void,
            p_color_blend_state: *const BlendState,
            p_dynamic_state: *const DynState,
            layout: VkPipelineLayout,
            render_pass: VkRenderPass,
            subpass: u32,
            base_pipeline_handle: VkPipeline,
            base_pipeline_index: i32,
        }
        let pipe_info = GfxPipeInfo {
            s_type: VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            stage_count: 2,
            p_stages: stages.as_ptr(),
            p_vertex_input_state: &vert_input,
            p_input_assembly_state: &input_asm,
            p_tessellation_state: std::ptr::null(),
            p_viewport_state: &vp_state,
            p_rasterization_state: &raster,
            p_multisample_state: &ms,
            p_depth_stencil_state: std::ptr::null(),
            p_color_blend_state: &blend,
            p_dynamic_state: &dyn_info,
            layout: pipe_layout,
            render_pass,
            subpass: 0,
            base_pipeline_handle: 0,
            base_pipeline_index: -1,
        };
        let mut pipe = 0u64;
        (lib.vkCreateGraphicsPipelines)(device, 0, 1, &pipe_info as *const _ as *const c_void, std::ptr::null(), &mut pipe);
        pipe
    }
}

pub fn shared_device_and_queue() -> (Arc<Device>, Arc<Queue>) {
    let mut guard = SHARED_VK.lock();
    if let Some((d, q)) = guard.as_ref() {
        return (d.clone(), q.clone());
    }
    let vk = unsafe { VulkanCore::try_init() };
    let dev = Arc::new(Device { vk: vk.clone() });
    let queue = Arc::new(Queue { vk });
    *guard = Some((dev.clone(), queue.clone()));
    (dev, queue)
}

pub fn shared_pipelines(device: &Device, format: TextureFormat) -> RenderPipelines {
    let mut guard = SHARED_PIPES.lock();
    if let Some((fmt, ref pipes)) = *guard
        && fmt == format
    {
        return pipes.clone();
    }
    let pipes = if let Some(vk) = &device.vk {
        unsafe {
            let rp_clear = create_render_pass(&vk.lib, vk.device, format, true);
            let rp_load = create_render_pass(&vk.lib, vk.device, format, false);
            let cells = create_graphics_pipeline(
                &vk.lib,
                vk.device,
                rp_clear,
                vk.cells_pipe_layout,
                vk.shader_mod,
                c"vs_main",
                c"fs_main",
                12,
                &[(0, VK_FORMAT_R32_UINT, 0), (1, VK_FORMAT_R32_UINT, 4), (2, VK_FORMAT_R32_UINT, 8)],
                false,
            );
            let corner = create_graphics_pipeline(
                &vk.lib,
                vk.device,
                rp_load,
                vk.cells_pipe_layout,
                vk.shader_mod,
                c"vs_corner",
                c"fs_corner",
                0,
                &[],
                true,
            );
            let image = create_graphics_pipeline(
                &vk.lib,
                vk.device,
                rp_clear,
                vk.image_pipe_layout,
                vk.image_mod,
                c"vs_main",
                c"fs_main",
                32,
                &[(0, VK_FORMAT_R32G32B32A32_SFLOAT, 0), (1, VK_FORMAT_R32G32B32A32_SFLOAT, 16)],
                false,
            );
            RenderPipelines {
                cells: ObjPtr(cells),
                corner: ObjPtr(corner),
                image: ObjPtr(image),
                render_pass_clear: rp_clear,
                render_pass_load: rp_load,
            }
        }
    } else {
        RenderPipelines {
            cells: ObjPtr(1),
            corner: ObjPtr(2),
            image: ObjPtr(3),
            render_pass_clear: 0,
            render_pass_load: 0,
        }
    };
    *guard = Some((format, pipes.clone()));
    pipes
}

impl Device {
    /// Custom shaders need GLSL to SPIR-V translation and offscreen targets,
    /// which this backend does not have. Returning a handle here made the
    /// renderer bind it as a real pipeline.
    pub fn create_postfx_pipeline(&self, _msl_src: &str, _format: TextureFormat) -> Result<ObjPtr, String> {
        Err("custom shaders are not supported on Linux yet".into())
    }

    #[inline]
    pub fn nearest_sampler(&self) -> SamplerHandle {
        self.vk.as_ref().map_or(0, |v| v.nearest_sampler)
    }

    #[inline]
    pub fn linear_sampler(&self) -> SamplerHandle {
        self.vk.as_ref().map_or(0, |v| v.linear_sampler)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_shaders_are_rejected() {
        let device = Device { vk: None };
        let err = device.create_postfx_pipeline("void mainImage() {}", TextureFormat::Bgra8Unorm).err();
        assert_eq!(err.as_deref(), Some("custom shaders are not supported on Linux yet"));
    }

    #[test]
    fn instance_extensions_take_only_what_the_loader_offers() {
        // A driver build without Wayland support still gets X11 surfaces.
        let x11_only = [c"VK_KHR_surface", c"VK_KHR_xlib_surface", c"VK_KHR_xcb_surface", c"VK_EXT_debug_utils"];
        assert_eq!(
            instance_extensions(&x11_only),
            Some(vec![c"VK_KHR_surface", c"VK_KHR_xlib_surface", c"VK_KHR_xcb_surface"])
        );
        let wayland_only = [c"VK_KHR_wayland_surface", c"VK_KHR_surface"];
        assert_eq!(instance_extensions(&wayland_only), Some(vec![c"VK_KHR_surface", c"VK_KHR_wayland_surface"]));
    }

    #[test]
    fn instance_extensions_need_a_surface_and_a_window_system() {
        assert_eq!(instance_extensions(&[c"VK_KHR_xlib_surface"]), None, "no VK_KHR_surface");
        assert_eq!(instance_extensions(&[c"VK_KHR_surface"]), None, "headless driver");
        assert_eq!(instance_extensions(&[]), None);
    }

    #[test]
    fn graphics_family_skips_families_that_cannot_draw() {
        let family = |queue_flags, queue_count| VkQueueFamilyProperties { queue_flags, queue_count, ..Default::default() };
        // Compute-only first, then an empty graphics family, then a usable one.
        let families = [family(2, 1), family(VK_QUEUE_GRAPHICS_BIT, 0), family(VK_QUEUE_GRAPHICS_BIT | 2, 4)];
        assert_eq!(graphics_family(&families), Some(2));
        assert_eq!(graphics_family(&families[..2]), None);
        assert_eq!(graphics_family(&[]), None);
    }
}
