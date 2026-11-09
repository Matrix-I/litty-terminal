//! Shared GPU device, queue, and pipeline helpers for every window.

use std::sync::Arc;

use crate::render::backend::{
    self, Device, HostBuffer, Queue, RenderPipelines, TextureFormat,
};

use super::{Instance, Viewport};

pub(super) fn make_viewport_buffer(device: &Device, panes: usize) -> HostBuffer<Viewport> {
    HostBuffer::new(device, panes.max(1))
}

pub(super) fn make_instance_buf(device: &Device, cells: usize) -> HostBuffer<Instance> {
    HostBuffer::new(device, cells.max(1))
}

/// The process-wide device and queue; created by the startup prewarm thread
/// (macOS) or the first window, whichever asks first.
pub(super) fn shared_device() -> (Arc<Device>, Arc<Queue>) {
    backend::shared_device_and_queue()
}

/// The cell, corner, and image pipelines for `format`.
pub(super) fn shared_pipeline(device: &Device, format: TextureFormat) -> RenderPipelines {
    backend::shared_pipelines(device, format)
}
