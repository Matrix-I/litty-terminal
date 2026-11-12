//! Direct Vulkan 1.0+ GPU backend on Linux (`libvulkan.so.1` via `dlopen`/`dlsym`),
//! with an automatic headless CPU fallback when running unit tests without a GPU.
//!
//! Split into focused submodules:
//! - `ffi`: Vulkan C ABI types, constants, and dynamically loaded function table (`VkLib`).
//! - `device`: `VulkanCore`, `Device`, `Queue`, `RenderPipelines`, and SPIR-V shader loading.
//! - `resources`: Host-visible coherent `HostBuffer<T>` and 2D `Texture`.
//! - `surface`: Wayland / X11 `VkSurfaceKHR`, `VkSwapchainKHR` (`FIFO`, `minImageCount = 2`), and `shrink_drawables`.
//! - `pass`: `CommandEncoder` and `RenderPass` draw encoding.

mod device;
mod ffi;
mod pass;
mod resources;
mod surface;

pub use device::{shared_device_and_queue, shared_pipelines, Device, Queue, RenderPipelines, CUSTOM_SHADERS_SUPPORTED};
pub use ffi::ObjPtr;
pub use pass::{CommandEncoder, RenderPass};
pub use resources::{HostBuffer, Texture};
pub use surface::{Surface, SurfaceFrame};
