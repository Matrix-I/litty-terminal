//! Minimal cross-platform GPU backend interface (`Metal` on macOS, `Vulkan` on Linux).
//!
//! Litty's instanced renderer needs only:
//! 1. Device, queue, and swapchain surface lifecycle (`create_surface`, `configure`, `shrink_drawables`).
//! 2. 2D textures (`R8Unorm`, `Rgba8Unorm`, `Bgra8Unorm`) with direct rectangle upload (`write_rect`).
//! 3. Host-visible unified/coherent buffers (`HostBuffer<T>`) for zero-copy dirty-row writes.
//! 4. Three core pipelines: instanced cells (`vs_main`/`fs_main`), rounded corners (`vs_corner`/`fs_corner`),
//!    and inline images (`vs_image`/`fs_image`) plus custom post-processing passes (`PostFx`).

#[cfg(target_os = "macos")]
mod metal;
#[cfg(target_os = "macos")]
pub use metal::*;

#[cfg(not(target_os = "macos"))]
mod linux_gpu;
#[cfg(not(target_os = "macos"))]
pub use linux_gpu::*;

/// Supported 2D texture and surface pixel formats.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TextureFormat {
    Bgra8Unorm,
    Rgba8Unorm,
    R8Unorm,
}

impl TextureFormat {
    #[inline]
    pub const fn bytes_per_pixel(self) -> u32 {
        match self {
            Self::Bgra8Unorm | Self::Rgba8Unorm => 4,
            Self::R8Unorm => 1,
        }
    }
}

/// Surface configuration applied on creation, resize, or transparency toggle.
#[derive(Copy, Clone, Debug)]
pub struct SurfaceConfig {
    pub format: TextureFormat,
    pub width: u32,
    pub height: u32,
    pub transparent: bool,
}
