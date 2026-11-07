//! Direct Metal backend on macOS via the zero-dependency Objective-C runtime (`objc_msgSend`).
//!
//! Split into focused submodules:
//! - `ffi`: Objective-C message send helpers, reference-counted `ObjPtr`, and Metal C types/constants.
//! - `device`: `Device`, `Queue`, `RenderPipelines`, and `shader.metal` compilation.
//! - `resources`: Unified-memory `HostBuffer<T>` and 2D `Texture`.
//! - `surface`: `CAMetalLayer` swapchain (`Surface`, `SurfaceFrame`) and `shrink_drawables`.
//! - `pass`: `CommandEncoder` and `RenderPass` draw encoding.

mod device;
mod ffi;
mod pass;
mod resources;
mod surface;

pub use device::{shared_device_and_queue, shared_pipelines, Device, Queue, RenderPipelines};
pub use ffi::ObjPtr;
pub use pass::{CommandEncoder, RenderPass};
pub use resources::{HostBuffer, Texture};
pub use surface::{Surface, SurfaceFrame};
