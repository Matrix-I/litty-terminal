//! Unified-memory `HostBuffer<T>` (`MTLResourceStorageModeShared`) and 2D `Texture` (`MTLTexture`).

use std::ffi::c_void;
use std::marker::PhantomData;

use super::device::{Device, Queue};
use super::ffi::*;
use crate::render::backend::TextureFormat;

/// Unified-memory (`MTLResourceStorageModeShared`) buffer mapped directly into CPU address space.
///
/// Allows `Renderer` to write dirty rows directly into `[MTLBuffer contents]` without a
/// duplicate `Vec<T>` on the heap or staging copies.
pub struct HostBuffer<T: bytemuck::Pod> {
    raw: ObjPtr,
    ptr: *mut T,
    len: usize,
    cap: usize,
    _marker: PhantomData<T>,
}

// SAFETY: `HostBuffer` owns its `MTLBuffer` allocation; access is synchronized through `&self`/`&mut self`.
unsafe impl<T: bytemuck::Pod + Send> Send for HostBuffer<T> {}
unsafe impl<T: bytemuck::Pod + Sync> Sync for HostBuffer<T> {}

impl<T: bytemuck::Pod> HostBuffer<T> {
    pub fn new(device: &Device, capacity: usize) -> Self {
        let cap = capacity.max(1);
        let elem_size = std::mem::size_of::<T>().max(1);
        let byte_len = cap * elem_size;
        unsafe {
            let raw = msg_send!(
                device.raw.as_ptr(),
                c"newBufferWithLength:options:",
                byte_len => usize,
                MTL_STORAGE_MODE_SHARED => usize;
                -> Id
            );
            assert!(!raw.is_null(), "newBufferWithLength failed ({byte_len} bytes)");
            let ptr = msg_send!(raw, c"contents"; -> *mut T);
            assert!(!ptr.is_null(), "MTLBuffer contents returned null");
            std::ptr::write_bytes(ptr as *mut u8, 0, byte_len);
            Self {
                raw: ObjPtr(raw),
                ptr,
                len: 0,
                cap,
                _marker: PhantomData,
            }
        }
    }

    pub fn from_slice(device: &Device, data: &[T]) -> Self {
        let mut buf = Self::new(device, data.len().max(1));
        buf.len = data.len();
        if !data.is_empty() {
            buf.as_mut_slice().copy_from_slice(data);
        }
        buf
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.cap
    }

    /// Resize logical length in place, or reallocate the shared `MTLBuffer` when capacity is exceeded
    /// (or when shrinking an oversized buffer). Returns `true` if a new buffer was allocated.
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

    /// Flush dirty element range `[start..end]` to the GPU (no-op on macOS unified memory).
    #[inline]
    pub fn flush_range(&self, _queue: &Queue, _start: usize, _end: usize) {}

    #[inline]
    pub fn raw_ptr(&self) -> Id {
        self.raw.as_ptr()
    }
}

/// 2D GPU texture (`MTLTexture`).
#[derive(Clone)]
pub struct Texture {
    raw: ObjPtr,
    pub width: u32,
    pub height: u32,
    pub format: TextureFormat,
}

impl Texture {
    pub fn new(device: &Device, width: u32, height: u32, format: TextureFormat, render_target: bool) -> Self {
        let w = width.max(1) as usize;
        let h = height.max(1) as usize;
        unsafe {
            let cls = class(c"MTLTextureDescriptor").expect("MTLTextureDescriptor");
            let desc = msg_send!(
                cls,
                c"texture2DDescriptorWithPixelFormat:width:height:mipmapped:",
                mtl_pixel_format(format) => usize,
                w => usize,
                h => usize,
                false => bool;
                -> Id
            );
            let mut usage = MTL_TEXTURE_USAGE_SHADER_READ;
            if render_target {
                usage |= MTL_TEXTURE_USAGE_RENDER_TARGET;
            }
            msg_send!(desc, c"setUsage:", usage => usize);
            msg_send!(desc, c"setStorageMode:", MTL_STORAGE_MODE_SHARED => usize);
            let raw = msg_send!(device.raw.as_ptr(), c"newTextureWithDescriptor:", desc => Id; -> Id);
            assert!(!raw.is_null(), "newTextureWithDescriptor failed ({w}x{h})");
            Self {
                raw: ObjPtr(raw),
                width: w as u32,
                height: h as u32,
                format,
            }
        }
    }

    /// Upload pixel bytes directly into a sub-rectangle of the texture.
    pub fn write_rect(&self, _queue: &Queue, x: u32, y: u32, width: u32, height: u32, data: &[u8], bytes_per_row: u32) {
        if width == 0 || height == 0 || data.is_empty() {
            return;
        }
        let region = MTLRegion {
            origin: MTLOrigin { x: x as usize, y: y as usize, z: 0 },
            size: MTLSize { width: width as usize, height: height as usize, depth: 1 },
        };
        unsafe {
            msg_send!(
                self.raw.as_ptr(),
                c"replaceRegion:mipmapLevel:withBytes:bytesPerRow:",
                region => MTLRegion,
                0usize => usize,
                data.as_ptr() as *const c_void => *const c_void,
                bytes_per_row as usize => usize
            );
        }
    }

    /// Read back RGBA8/BGRA8 pixels from a shared texture into `dst` (`bytes_per_row * height`).
    pub fn read_pixels(&self, dst: &mut [u8], bytes_per_row: u32) {
        let region = MTLRegion {
            origin: MTLOrigin { x: 0, y: 0, z: 0 },
            size: MTLSize { width: self.width as usize, height: self.height as usize, depth: 1 },
        };
        unsafe {
            msg_send!(
                self.raw.as_ptr(),
                c"getBytes:bytesPerRow:fromRegion:mipmapLevel:",
                dst.as_mut_ptr() as *mut c_void => *mut c_void,
                bytes_per_row as usize => usize,
                region => MTLRegion,
                0usize => usize
            );
        }
    }

    #[inline]
    pub fn raw_ptr(&self) -> Id {
        self.raw.as_ptr()
    }
}
