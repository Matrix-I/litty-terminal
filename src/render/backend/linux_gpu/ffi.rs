//! Vulkan 1.0/1.1 C ABI types, constants, and dynamically loaded function table (`VkLib`).

use std::ffi::{c_char, c_void};

use crate::render::backend::TextureFormat;

pub type RawTexHandle = u64;
pub type SamplerHandle = u64;

#[derive(Clone, Default)]
pub struct ObjPtr(pub u64);

pub(super) type VkFlags = u32;
pub(super) type VkBool32 = u32;
pub(super) type VkDeviceSize = u64;
pub(super) type VkInstance = *mut c_void;
pub(super) type VkPhysicalDevice = *mut c_void;
pub(super) type VkDevice = *mut c_void;
pub(super) type VkQueue = *mut c_void;
pub(super) type VkSurfaceKHR = u64;
pub(super) type VkSwapchainKHR = u64;
pub(super) type VkImage = u64;
pub(super) type VkImageView = u64;
pub(super) type VkBuffer = u64;
pub(super) type VkDeviceMemory = u64;
pub(super) type VkShaderModule = u64;
pub(super) type VkDescriptorSetLayout = u64;
pub(super) type VkPipelineLayout = u64;
pub(super) type VkRenderPass = u64;
pub(super) type VkPipeline = u64;
pub(super) type VkDescriptorPool = u64;
pub(super) type VkDescriptorSet = u64;
pub(super) type VkSampler = u64;
pub(super) type VkFramebuffer = u64;
pub(super) type VkCommandPool = u64;
pub(super) type VkCommandBuffer = *mut c_void;
pub(super) type VkSemaphore = u64;
pub(super) type VkFence = u64;

pub(super) const VK_SUCCESS: i32 = 0;
pub(super) const VK_SUBOPTIMAL_KHR: i32 = 1000001003;
pub(super) const VK_ERROR_OUT_OF_DATE_KHR: i32 = -1000001004;

/// Whether a Vulkan call returned `VK_SUCCESS`. Anything else is reported on
/// stderr: the object it was to create, or the work it was to queue, does
/// not exist, and callers must not use it.
pub(super) fn vk_ok(result: i32, what: &str) -> bool {
    if result == VK_SUCCESS {
        return true;
    }
    eprintln!("litty: {what} failed (VkResult {result})");
    false
}

/// Whether a `vkQueuePresentKHR` result is expected: success, or a
/// swapchain that no longer matches the window, which the next acquire
/// reports so the renderer rebuilds it. Anything else is reported.
pub(super) fn present_ok(result: i32) -> bool {
    matches!(result, VK_SUCCESS | VK_SUBOPTIMAL_KHR | VK_ERROR_OUT_OF_DATE_KHR) || vk_ok(result, "vkQueuePresentKHR")
}

pub(super) const VK_STRUCTURE_TYPE_APPLICATION_INFO: u32 = 0;
pub(super) const VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO: u32 = 1;
pub(super) const VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO: u32 = 2;
pub(super) const VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO: u32 = 3;
pub(super) const VK_STRUCTURE_TYPE_SUBMIT_INFO: u32 = 4;
pub(super) const VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO: u32 = 5;
pub(super) const VK_STRUCTURE_TYPE_FENCE_CREATE_INFO: u32 = 8;
pub(super) const VK_STRUCTURE_TYPE_SEMAPHORE_CREATE_INFO: u32 = 9;
pub(super) const VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO: u32 = 12;
pub(super) const VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO: u32 = 14;
pub(super) const VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO: u32 = 15;
pub(super) const VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO: u32 = 16;
pub(super) const VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO: u32 = 18;
pub(super) const VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO: u32 = 19;
pub(super) const VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO: u32 = 20;
pub(super) const VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO: u32 = 22;
pub(super) const VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO: u32 = 23;
pub(super) const VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO: u32 = 24;
pub(super) const VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO: u32 = 26;
pub(super) const VK_STRUCTURE_TYPE_PIPELINE_DYNAMIC_STATE_CREATE_INFO: u32 = 27;
pub(super) const VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO: u32 = 28;
pub(super) const VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO: u32 = 30;
pub(super) const VK_STRUCTURE_TYPE_SAMPLER_CREATE_INFO: u32 = 31;
pub(super) const VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO: u32 = 32;
pub(super) const VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO: u32 = 33;
pub(super) const VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO: u32 = 34;
pub(super) const VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET: u32 = 35;
pub(super) const VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO: u32 = 37;
pub(super) const VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO: u32 = 38;
pub(super) const VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO: u32 = 39;
pub(super) const VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO: u32 = 40;
pub(super) const VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO: u32 = 42;
pub(super) const VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO: u32 = 43;
pub(super) const VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER: u32 = 45;
// Read-back only (`Texture::read_pixels`, GPU tests).
#[cfg_attr(not(test), allow(dead_code))]
pub(super) const VK_STRUCTURE_TYPE_MEMORY_BARRIER: u32 = 46;
pub(super) const VK_STRUCTURE_TYPE_SWAPCHAIN_CREATE_INFO_KHR: u32 = 1000001000;
pub(super) const VK_STRUCTURE_TYPE_PRESENT_INFO_KHR: u32 = 1000001001;
pub(super) const VK_STRUCTURE_TYPE_XLIB_SURFACE_CREATE_INFO_KHR: u32 = 1000004000;
pub(super) const VK_STRUCTURE_TYPE_XCB_SURFACE_CREATE_INFO_KHR: u32 = 1000005000;
pub(super) const VK_STRUCTURE_TYPE_WAYLAND_SURFACE_CREATE_INFO_KHR: u32 = 1000006000;

pub(super) const VK_FORMAT_R8_UNORM: u32 = 9;
pub(super) const VK_FORMAT_R8G8B8A8_UNORM: u32 = 37;
pub(super) const VK_FORMAT_B8G8R8A8_UNORM: u32 = 44;

pub(super) const VK_IMAGE_LAYOUT_UNDEFINED: u32 = 0;
pub(super) const VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL: u32 = 2;
pub(super) const VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL: u32 = 5;
pub(super) const VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL: u32 = 6;
pub(super) const VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL: u32 = 7;
pub(super) const VK_IMAGE_LAYOUT_PRESENT_SRC_KHR: u32 = 1000001002;

pub(super) const VK_ATTACHMENT_LOAD_OP_LOAD: u32 = 0;
pub(super) const VK_ATTACHMENT_LOAD_OP_CLEAR: u32 = 1;
pub(super) const VK_ATTACHMENT_LOAD_OP_DONT_CARE: u32 = 2;
pub(super) const VK_ATTACHMENT_STORE_OP_STORE: u32 = 0;

pub(super) const VK_PRIMITIVE_TOPOLOGY_TRIANGLE_STRIP: u32 = 4;
pub(super) const VK_VERTEX_INPUT_RATE_INSTANCE: u32 = 1;
pub(super) const VK_FORMAT_R32_UINT: u32 = 98;
pub(super) const VK_FORMAT_R32G32B32A32_SFLOAT: u32 = 109;

pub(super) const VK_SHADER_STAGE_VERTEX_BIT: u32 = 0x0000_0001;
pub(super) const VK_SHADER_STAGE_FRAGMENT_BIT: u32 = 0x0000_0010;
pub(super) const VK_SHADER_STAGE_ALL_GRAPHICS: u32 = 0x0000_001F;

pub(super) const VK_DESCRIPTOR_TYPE_SAMPLER: u32 = 0;
pub(super) const VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE: u32 = 2;
pub(super) const VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER: u32 = 6;
pub(super) const VK_DESCRIPTOR_TYPE_STORAGE_BUFFER: u32 = 7;
pub(super) const VK_DESCRIPTOR_POOL_CREATE_FREE_DESCRIPTOR_SET_BIT: u32 = 0x0000_0001;

pub(super) const VK_BUFFER_USAGE_TRANSFER_SRC_BIT: u32 = 0x0000_0001;
// Read-back only (`Texture::read_pixels`, GPU tests).
#[cfg_attr(not(test), allow(dead_code))]
pub(super) const VK_BUFFER_USAGE_TRANSFER_DST_BIT: u32 = 0x0000_0002;
pub(super) const VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT: u32 = 0x0000_0010;
pub(super) const VK_BUFFER_USAGE_STORAGE_BUFFER_BIT: u32 = 0x0000_0020;
pub(super) const VK_BUFFER_USAGE_VERTEX_BUFFER_BIT: u32 = 0x0000_0080;

pub(super) const VK_IMAGE_USAGE_TRANSFER_SRC_BIT: u32 = 0x0000_0001;
pub(super) const VK_IMAGE_USAGE_TRANSFER_DST_BIT: u32 = 0x0000_0002;
pub(super) const VK_IMAGE_USAGE_SAMPLED_BIT: u32 = 0x0000_0004;
pub(super) const VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT: u32 = 0x0000_0010;

pub(super) const VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT: u32 = 0x0000_0002;
pub(super) const VK_MEMORY_PROPERTY_HOST_COHERENT_BIT: u32 = 0x0000_0004;

pub(super) const VK_BLEND_FACTOR_ZERO: u32 = 0;
pub(super) const VK_BLEND_FACTOR_ONE: u32 = 1;
pub(super) const VK_BLEND_FACTOR_SRC_ALPHA: u32 = 6;
pub(super) const VK_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA: u32 = 7;
pub(super) const VK_BLEND_OP_ADD: u32 = 0;

pub(super) const VK_DYNAMIC_STATE_VIEWPORT: u32 = 0;
pub(super) const VK_DYNAMIC_STATE_SCISSOR: u32 = 1;

pub(super) const VK_FILTER_NEAREST: u32 = 0;
pub(super) const VK_FILTER_LINEAR: u32 = 1;
pub(super) const VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE: u32 = 2;

pub(super) const VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR: u32 = 0x0000_0001;
pub(super) const VK_COMPOSITE_ALPHA_PRE_MULTIPLIED_BIT_KHR: u32 = 0x0000_0002;
pub(super) const VK_COMPOSITE_ALPHA_POST_MULTIPLIED_BIT_KHR: u32 = 0x0000_0004;
pub(super) const VK_COMPOSITE_ALPHA_INHERIT_BIT_KHR: u32 = 0x0000_0008;
pub(super) const VK_PRESENT_MODE_FIFO_KHR: u32 = 2;

pub(super) const VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT: u32 = 0x0000_0001;
pub(super) const VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT: u32 = 0x0000_0080;
pub(super) const VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT: u32 = 0x0000_0400;
pub(super) const VK_PIPELINE_STAGE_TRANSFER_BIT: u32 = 0x0000_1000;
// Read-back only (`Texture::read_pixels`, GPU tests).
#[cfg_attr(not(test), allow(dead_code))]
pub(super) const VK_PIPELINE_STAGE_HOST_BIT: u32 = 0x0000_4000;

pub(super) const VK_ACCESS_TRANSFER_WRITE_BIT: u32 = 0x0000_1000;
pub(super) const VK_ACCESS_SHADER_READ_BIT: u32 = 0x0000_0020;
// Read-back only (`Texture::read_pixels`, GPU tests).
#[cfg_attr(not(test), allow(dead_code))]
pub(super) const VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT: u32 = 0x0000_0100;
// Read-back only (`Texture::read_pixels`, GPU tests).
#[cfg_attr(not(test), allow(dead_code))]
pub(super) const VK_ACCESS_TRANSFER_READ_BIT: u32 = 0x0000_0800;
// Read-back only (`Texture::read_pixels`, GPU tests).
#[cfg_attr(not(test), allow(dead_code))]
pub(super) const VK_ACCESS_HOST_READ_BIT: u32 = 0x0000_2000;

#[inline]
pub(super) const fn vk_format(format: TextureFormat) -> u32 {
    match format {
        TextureFormat::R8Unorm => VK_FORMAT_R8_UNORM,
        TextureFormat::Rgba8Unorm => VK_FORMAT_R8G8B8A8_UNORM,
        TextureFormat::Bgra8Unorm => VK_FORMAT_B8G8R8A8_UNORM,
    }
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
pub(super) struct VkExtent2D {
    pub width: u32,
    pub height: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
pub(super) struct VkExtent3D {
    pub width: u32,
    pub height: u32,
    pub depth: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
pub(super) struct VkOffset2D {
    pub x: i32,
    pub y: i32,
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
pub(super) struct VkOffset3D {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
pub(super) struct VkRect2D {
    pub offset: VkOffset2D,
    pub extent: VkExtent2D,
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
pub(super) struct VkViewport {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub min_depth: f32,
    pub max_depth: f32,
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
pub(super) struct VkClearColorValue {
    pub float32: [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
pub(super) struct VkMemoryRequirements {
    pub size: VkDeviceSize,
    pub alignment: VkDeviceSize,
    pub memory_type_bits: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
pub(super) struct VkMemoryType {
    pub property_flags: VkFlags,
    pub heap_index: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
pub(super) struct VkMemoryHeap {
    pub size: VkDeviceSize,
    pub flags: VkFlags,
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
pub(super) struct VkPhysicalDeviceMemoryProperties {
    pub memory_type_count: u32,
    pub memory_types: [VkMemoryType; 32],
    pub memory_heap_count: u32,
    pub memory_heaps: [VkMemoryHeap; 16],
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
pub(super) struct VkSurfaceCapabilitiesKHR {
    pub min_image_count: u32,
    pub max_image_count: u32,
    pub current_extent: VkExtent2D,
    pub min_image_extent: VkExtent2D,
    pub max_image_extent: VkExtent2D,
    pub max_image_array_layers: u32,
    pub supported_transforms: VkFlags,
    pub current_transform: VkFlags,
    pub supported_composite_alpha: VkFlags,
    pub supported_usage_flags: VkFlags,
}

pub(super) const VK_QUEUE_GRAPHICS_BIT: u32 = 0x0000_0001;

pub(super) const VK_PHYSICAL_DEVICE_TYPE_CPU: u32 = 4;

/// `VK_MAKE_API_VERSION(0, major, minor, 0)`.
pub(super) const fn vk_api_version(major: u32, minor: u32) -> u32 {
    (major << 22) | (minor << 12)
}
pub(super) const VK_API_VERSION_1_0: u32 = vk_api_version(1, 0);
pub(super) const VK_API_VERSION_1_1: u32 = vk_api_version(1, 1);

/// `VkPhysicalDeviceProperties`. Only the leading fields are read; `limits`
/// (504 bytes, 8-byte aligned) and `sparseProperties` (20 bytes) are kept as
/// opaque storage of the right size and alignment.
#[repr(C)]
#[derive(Copy, Clone)]
pub(super) struct VkPhysicalDeviceProperties {
    pub api_version: u32,
    pub driver_version: u32,
    pub vendor_id: u32,
    pub device_id: u32,
    pub device_type: u32,
    pub device_name: [c_char; 256],
    pub pipeline_cache_uuid: [u8; 16],
    pub limits_and_sparse: [u64; 66],
}

impl Default for VkPhysicalDeviceProperties {
    fn default() -> Self {
        Self {
            api_version: 0,
            driver_version: 0,
            vendor_id: 0,
            device_id: 0,
            device_type: 0,
            device_name: [0; 256],
            pipeline_cache_uuid: [0; 16],
            limits_and_sparse: [0; 66],
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone)]
pub(super) struct VkExtensionProperties {
    pub extension_name: [c_char; 256],
    pub spec_version: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
pub(super) struct VkQueueFamilyProperties {
    pub queue_flags: VkFlags,
    pub queue_count: u32,
    pub timestamp_valid_bits: u32,
    pub min_image_transfer_granularity: VkExtent3D,
}

/// Dynamically loaded Vulkan function table (`libvulkan.so.1`).
#[allow(non_snake_case, dead_code)]
pub(super) struct VkLib {
    pub _handle: *mut c_void,
    pub vkGetInstanceProcAddr: unsafe extern "C" fn(VkInstance, *const c_char) -> *const c_void,
    pub vkCreateInstance: unsafe extern "C" fn(*const c_void, *const c_void, *mut VkInstance) -> i32,
    pub vkEnumeratePhysicalDevices: unsafe extern "C" fn(VkInstance, *mut u32, *mut VkPhysicalDevice) -> i32,
    pub vkGetPhysicalDeviceMemoryProperties: unsafe extern "C" fn(VkPhysicalDevice, *mut VkPhysicalDeviceMemoryProperties),
    pub vkCreateDevice: unsafe extern "C" fn(VkPhysicalDevice, *const c_void, *const c_void, *mut VkDevice) -> i32,
    pub vkGetDeviceQueue: unsafe extern "C" fn(VkDevice, u32, u32, *mut VkQueue),
    pub vkCreateBuffer: unsafe extern "C" fn(VkDevice, *const c_void, *const c_void, *mut VkBuffer) -> i32,
    pub vkDestroyBuffer: unsafe extern "C" fn(VkDevice, VkBuffer, *const c_void),
    pub vkGetBufferMemoryRequirements: unsafe extern "C" fn(VkDevice, VkBuffer, *mut VkMemoryRequirements),
    pub vkAllocateMemory: unsafe extern "C" fn(VkDevice, *const c_void, *const c_void, *mut VkDeviceMemory) -> i32,
    pub vkFreeMemory: unsafe extern "C" fn(VkDevice, VkDeviceMemory, *const c_void),
    pub vkBindBufferMemory: unsafe extern "C" fn(VkDevice, VkBuffer, VkDeviceMemory, VkDeviceSize) -> i32,
    pub vkMapMemory: unsafe extern "C" fn(VkDevice, VkDeviceMemory, VkDeviceSize, VkDeviceSize, VkFlags, *mut *mut c_void) -> i32,
    pub vkCreateImage: unsafe extern "C" fn(VkDevice, *const c_void, *const c_void, *mut VkImage) -> i32,
    pub vkDestroyImage: unsafe extern "C" fn(VkDevice, VkImage, *const c_void),
    pub vkGetImageMemoryRequirements: unsafe extern "C" fn(VkDevice, VkImage, *mut VkMemoryRequirements),
    pub vkBindImageMemory: unsafe extern "C" fn(VkDevice, VkImage, VkDeviceMemory, VkDeviceSize) -> i32,
    pub vkCreateImageView: unsafe extern "C" fn(VkDevice, *const c_void, *const c_void, *mut VkImageView) -> i32,
    pub vkDestroyImageView: unsafe extern "C" fn(VkDevice, VkImageView, *const c_void),
    pub vkCreateSampler: unsafe extern "C" fn(VkDevice, *const c_void, *const c_void, *mut VkSampler) -> i32,
    pub vkCreateShaderModule: unsafe extern "C" fn(VkDevice, *const c_void, *const c_void, *mut VkShaderModule) -> i32,
    pub vkCreateDescriptorSetLayout: unsafe extern "C" fn(VkDevice, *const c_void, *const c_void, *mut VkDescriptorSetLayout) -> i32,
    pub vkCreatePipelineLayout: unsafe extern "C" fn(VkDevice, *const c_void, *const c_void, *mut VkPipelineLayout) -> i32,
    pub vkCreateRenderPass: unsafe extern "C" fn(VkDevice, *const c_void, *const c_void, *mut VkRenderPass) -> i32,
    pub vkCreateGraphicsPipelines: unsafe extern "C" fn(VkDevice, u64, u32, *const c_void, *const c_void, *mut VkPipeline) -> i32,
    pub vkCreateDescriptorPool: unsafe extern "C" fn(VkDevice, *const c_void, *const c_void, *mut VkDescriptorPool) -> i32,
    pub vkAllocateDescriptorSets: unsafe extern "C" fn(VkDevice, *const c_void, *mut VkDescriptorSet) -> i32,
    pub vkUpdateDescriptorSets: unsafe extern "C" fn(VkDevice, u32, *const c_void, u32, *const c_void),
    pub vkFreeDescriptorSets: unsafe extern "C" fn(VkDevice, VkDescriptorPool, u32, *const VkDescriptorSet) -> i32,
    pub vkCreateFramebuffer: unsafe extern "C" fn(VkDevice, *const c_void, *const c_void, *mut VkFramebuffer) -> i32,
    pub vkDestroyFramebuffer: unsafe extern "C" fn(VkDevice, VkFramebuffer, *const c_void),
    pub vkCreateCommandPool: unsafe extern "C" fn(VkDevice, *const c_void, *const c_void, *mut VkCommandPool) -> i32,
    pub vkAllocateCommandBuffers: unsafe extern "C" fn(VkDevice, *const c_void, *mut VkCommandBuffer) -> i32,
    pub vkResetCommandBuffer: unsafe extern "C" fn(VkCommandBuffer, VkFlags) -> i32,
    pub vkBeginCommandBuffer: unsafe extern "C" fn(VkCommandBuffer, *const c_void) -> i32,
    pub vkEndCommandBuffer: unsafe extern "C" fn(VkCommandBuffer) -> i32,
    pub vkCmdBeginRenderPass: unsafe extern "C" fn(VkCommandBuffer, *const c_void, u32),
    pub vkCmdEndRenderPass: unsafe extern "C" fn(VkCommandBuffer),
    pub vkCmdBindPipeline: unsafe extern "C" fn(VkCommandBuffer, u32, VkPipeline),
    pub vkCmdBindDescriptorSets: unsafe extern "C" fn(VkCommandBuffer, u32, VkPipelineLayout, u32, u32, *const VkDescriptorSet, u32, *const u32),
    pub vkCmdBindVertexBuffers: unsafe extern "C" fn(VkCommandBuffer, u32, u32, *const VkBuffer, *const VkDeviceSize),
    pub vkCmdSetViewport: unsafe extern "C" fn(VkCommandBuffer, u32, u32, *const VkViewport),
    pub vkCmdSetScissor: unsafe extern "C" fn(VkCommandBuffer, u32, u32, *const VkRect2D),
    pub vkCmdDraw: unsafe extern "C" fn(VkCommandBuffer, u32, u32, u32, u32),
    pub vkCmdPipelineBarrier: unsafe extern "C" fn(VkCommandBuffer, VkFlags, VkFlags, VkFlags, u32, *const c_void, u32, *const c_void, u32, *const c_void),
    pub vkCmdCopyBufferToImage: unsafe extern "C" fn(VkCommandBuffer, VkBuffer, VkImage, u32, u32, *const c_void),
    pub vkCmdCopyImageToBuffer: unsafe extern "C" fn(VkCommandBuffer, VkImage, u32, VkBuffer, u32, *const c_void),
    pub vkQueueSubmit: unsafe extern "C" fn(VkQueue, u32, *const c_void, VkFence) -> i32,
    pub vkQueueWaitIdle: unsafe extern "C" fn(VkQueue) -> i32,
    pub vkDeviceWaitIdle: unsafe extern "C" fn(VkDevice) -> i32,
    pub vkCreateSemaphore: unsafe extern "C" fn(VkDevice, *const c_void, *const c_void, *mut VkSemaphore) -> i32,
    pub vkCreateFence: unsafe extern "C" fn(VkDevice, *const c_void, *const c_void, *mut VkFence) -> i32,
    pub vkWaitForFences: unsafe extern "C" fn(VkDevice, u32, *const VkFence, VkBool32, u64) -> i32,
    pub vkResetFences: unsafe extern "C" fn(VkDevice, u32, *const VkFence) -> i32,
    pub vkDestroySemaphore: unsafe extern "C" fn(VkDevice, VkSemaphore, *const c_void),
    pub vkDestroyFence: unsafe extern "C" fn(VkDevice, VkFence, *const c_void),
    pub vkCreateWaylandSurfaceKHR: Option<unsafe extern "C" fn(VkInstance, *const c_void, *const c_void, *mut VkSurfaceKHR) -> i32>,
    pub vkCreateXlibSurfaceKHR: Option<unsafe extern "C" fn(VkInstance, *const c_void, *const c_void, *mut VkSurfaceKHR) -> i32>,
    pub vkCreateXcbSurfaceKHR: Option<unsafe extern "C" fn(VkInstance, *const c_void, *const c_void, *mut VkSurfaceKHR) -> i32>,
    pub vkGetPhysicalDeviceSurfaceSupportKHR: unsafe extern "C" fn(VkPhysicalDevice, u32, VkSurfaceKHR, *mut VkBool32) -> i32,
    pub vkGetPhysicalDeviceSurfaceCapabilitiesKHR: unsafe extern "C" fn(VkPhysicalDevice, VkSurfaceKHR, *mut VkSurfaceCapabilitiesKHR) -> i32,
    pub vkCreateSwapchainKHR: unsafe extern "C" fn(VkDevice, *const c_void, *const c_void, *mut VkSwapchainKHR) -> i32,
    pub vkDestroySwapchainKHR: unsafe extern "C" fn(VkDevice, VkSwapchainKHR, *const c_void),
    pub vkGetSwapchainImagesKHR: unsafe extern "C" fn(VkDevice, VkSwapchainKHR, *mut u32, *mut VkImage) -> i32,
    pub vkAcquireNextImageKHR: unsafe extern "C" fn(VkDevice, VkSwapchainKHR, u64, VkSemaphore, VkFence, *mut u32) -> i32,
    pub vkQueuePresentKHR: unsafe extern "C" fn(VkQueue, *const c_void) -> i32,
    pub vkDestroySurfaceKHR: unsafe extern "C" fn(VkInstance, VkSurfaceKHR, *const c_void),
}

unsafe impl Send for VkLib {}
unsafe impl Sync for VkLib {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_vk_success_counts_as_created() {
        assert!(vk_ok(VK_SUCCESS, "test"));
        // VK_ERROR_OUT_OF_DEVICE_MEMORY, VK_ERROR_DEVICE_LOST, VK_INCOMPLETE.
        assert!(!vk_ok(-2, "test"));
        assert!(!vk_ok(-4, "test"));
        assert!(!vk_ok(5, "test"));
    }

    #[test]
    fn a_stale_swapchain_is_an_expected_present_result() {
        assert!(present_ok(VK_SUCCESS));
        assert!(present_ok(VK_SUBOPTIMAL_KHR));
        assert!(present_ok(VK_ERROR_OUT_OF_DATE_KHR));
        assert!(!present_ok(-4), "device lost is reported");
    }
}
