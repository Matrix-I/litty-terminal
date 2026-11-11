# Review fixes C (Vulkan inline images and uploads) Implementation Plan — 2026-11-11

> **For agentic workers:** REQUIRED SUB-SKILL: development-superpowers:subagent-driven-development or executing-plans-strict. Steps use `- [ ]`.

**Goal:** Fix findings 1 and 9 of
[the 2026-11-11 review](../reviews/2026-11-11-linux-history-ai-graphics.md):
inline images (Kitty graphics, Sixel) are drawn with their own texture on
Linux, texture uploads no longer start from an `UNDEFINED` layout, and small
uploads reuse one staging buffer. Findings 2, 6, 7, 10 are in
[plan A](./2026-11-11-review-fixes-a-graphics-history-ai.md); findings 3, 4,
5, 8 are in [plan B](./2026-11-11-review-fixes-b-vulkan-robustness.md).

**Architecture:** Three changes inside `src/render/backend/linux_gpu/`, one
commit each:

1. A sampled texture is moved to `SHADER_READ_ONLY_OPTIMAL` when it is created, and every upload starts from that layout, so a partial upload keeps the rest of the image.
2. Every sampled texture owns one descriptor set for the image pipeline (uniforms, its view, the linear sampler), written once at creation; `draw_image_quad` binds it.
3. Uploads up to 4 MiB share one staging buffer owned by `VulkanCore`; larger ones keep a one-off buffer.

The public backend API used by the renderer (`Texture::new`, `Texture::write_rect`, `RenderPass::draw_image_quad`) keeps its signatures, so the macOS backend and the shared renderer are untouched.

**Tech Stack:** Rust 2024, Vulkan 1.0 through `dlopen`, no new dependency.

**Branch:** `fix/vulkan-inline-images` _(2026-11-12: stacked on `fix/vulkan-robustness` instead, so both can be tested on Linux before either is merged)_, created from `main` **after plan B is merged** (both plans edit `ffi.rs` and `device.rs`).

## Confirmed Requirements

1. Linux behaviour is verified by the user on a Linux machine. Tasks contain unit tests for pure logic, a compile check for the Linux target, and a manual checklist. (user, 2026-11-11)
2. The ten main findings are fixed in three independent plans, one branch each; this is plan C. (user, 2026-11-11)

## Global Constraints

From `CLAUDE.md` (project):

- Performance: under 50 MB idle RAM with all windows closed, 0% idle CPU, zero heap allocation on hot paths (render loop). Task 3 removes a buffer and memory allocation from every glyph upload; its retained buffer is capped at 4 MiB.
- Platform code: both targets must compile. Only Linux files change and no shared signature changes.
- Bug fixes start with a failing regression test. Where a test needs a live Vulkan device, the plan says so and gives a manual check.
- Scope: change only what the task needs.
- Verification before every commit that touches `src/`:
  - `cargo check`
  - `cargo check --target x86_64-unknown-linux-gnu`
  - `cargo test` (outside the sandbox; PTY tests require `openpty`)
  - `cargo clippy --all-targets -- -D warnings`
  - `git diff --check`
- Commit only when asked; never push. Conventional Commits `type(scope): summary`, lowercase, imperative, no trailing period, at most 72 characters. Stage files explicitly. No `Co-Authored-By` trailer.

Supported Linux target: x86_64 (including Arch Linux).

**The tests in `linux_gpu/` compile and run only on Linux.** On macOS the
executor proves they compile with
`cargo check --tests --target x86_64-unknown-linux-gnu` and reports them as
"not run here"; the user runs `cargo test --lib linux_gpu` on Linux.

**Ordering rule the code relies on:** `Texture::new` and `Texture::write_rect`
record into the shared `upload_cmd` and must not be called between
`CommandEncoder::new` and `present_and_submit`. Today every call site obeys it
(`src/render/renderer/frame.rs:525` uploads before line 527 creates the
encoder; the only texture created later is the custom-shader target, a render
target that records nothing, and custom shaders are off on Linux after plan B).

## Technical Defaults

- The image descriptor set uses the linear sampler, which is what `ImageLayer::draw` passes today (`src/render/image_layer.rs:130`); the `_sampler` argument stays unused.
- The image descriptor pool holds 2048 sets (one per sampled texture). A texture created when the pool is full gets no set and its image is skipped, not drawn wrongly.
- The shared staging buffer is at least 64 KiB, grows by powers of two, and is never kept above 4 MiB; a 64 MiB inline image still uses a one-off buffer so idle RAM does not keep it.
- `draw_corners` binds cells descriptor set 0 itself, because an image draw before it leaves an image-layout set bound.

## Risks & Rollback

| Risk | Mitigation | Revert |
|---|---|---|
| A wrong layout transition corrupts glyph or image textures | Pure-function tests for the transition source; manual text and image checks on Linux | Revert the Task 1 commit |
| Descriptor pool exhaustion with thousands of live image textures | Missing set means the image is skipped; pool sized above `MAX_IMAGES` (1024) per terminal | Revert the Task 2 commit |
| The retained staging buffer raises idle RAM | Capped at 4 MiB, measured in Task 4's checklist against the 50 MB budget | Revert the Task 3 commit |
| Several windows or tabs together hold more than 2048 sampled textures | The extra images are skipped, not drawn wrongly; raising `IMAGE_DESCRIPTOR_SETS` is a one-line change | — |
| Plans A, B and C each add status lines to the review file on separate branches | Expect a trivial merge conflict there; keep all status lines | — |
| None of this can run on the macOS development machine | Compile check for the Linux target plus the manual checklist in Task 4; every unverified item is reported | — |

No persisted data, config format, keybinding or public API changes.

---

## Task 1: Uploads start from the texture's real layout (finding 9, layout)

**Files:**
- Modify: `src/render/backend/linux_gpu/resources.rs:228-233` (`TextureInner`), `:260-368` (`Texture::new`), `:370-520` (`Texture::write_rect`)
- Test: `src/render/backend/linux_gpu/resources.rs` (new `tests` module at the end of the file)

**Interfaces:**
- Produces `fn upload_source(ready: bool) -> (u32, VkFlags, VkFlags)` returning `(old_layout, src_access_mask, src_stage_mask)`.
- Produces `unsafe fn begin_upload(vk: &VulkanCore)`, `unsafe fn submit_upload(vk: &VulkanCore)` and `unsafe fn image_barrier(vk: &VulkanCore, image: VkImage, layouts: (u32, u32), access: (VkFlags, VkFlags), stages: (VkFlags, VkFlags))` (all private to `resources.rs`).
- `TextureInner` gains `ready: bool` (the image is in `SHADER_READ_ONLY_OPTIMAL` between uploads).

- [x] **Step 1: Write the failing test**

Append to `resources.rs`:

```rust
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
}
```

- [x] **Step 2: Confirm the failure**

Run: `cargo check --tests --target x86_64-unknown-linux-gnu`
Expected: FAIL to compile with `cannot find function upload_source in this scope`.

- [x] **Step 3: Add the helpers**

In `resources.rs`, above `struct TextureInner`:

```rust
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

/// Start recording a one-off upload into the shared command buffer. Must not
/// run while a frame is being recorded (see `CommandEncoder`).
unsafe fn begin_upload(vk: &VulkanCore) {
    unsafe {
        #[repr(C)]
        struct BeginInfo {
            s_type: u32,
            p_next: *const c_void,
            flags: u32,
            p_inheritance_info: *const c_void,
        }
        let b_info = BeginInfo {
            s_type: VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
            p_next: std::ptr::null(),
            flags: 1,
            p_inheritance_info: std::ptr::null(),
        };
        (vk.lib.vkResetCommandBuffer)(vk.upload_cmd, 0);
        (vk.lib.vkBeginCommandBuffer)(vk.upload_cmd, &b_info as *const _ as *const c_void);
    }
}

/// Submit the recorded upload and wait for it, so its staging memory can be reused.
unsafe fn submit_upload(vk: &VulkanCore) {
    unsafe {
        (vk.lib.vkEndCommandBuffer)(vk.upload_cmd);
        #[repr(C)]
        struct SubmitInfo {
            s_type: u32,
            p_next: *const c_void,
            wait_semaphore_count: u32,
            p_wait_semaphores: *const VkSemaphore,
            p_wait_dst_stage_mask: *const VkFlags,
            command_buffer_count: u32,
            p_command_buffers: *const VkCommandBuffer,
            signal_semaphore_count: u32,
            p_signal_semaphores: *const VkSemaphore,
        }
        let s_info = SubmitInfo {
            s_type: VK_STRUCTURE_TYPE_SUBMIT_INFO,
            p_next: std::ptr::null(),
            wait_semaphore_count: 0,
            p_wait_semaphores: std::ptr::null(),
            p_wait_dst_stage_mask: std::ptr::null(),
            command_buffer_count: 1,
            p_command_buffers: &vk.upload_cmd,
            signal_semaphore_count: 0,
            p_signal_semaphores: std::ptr::null(),
        };
        (vk.lib.vkQueueSubmit)(vk.queue, 1, &s_info as *const _ as *const c_void, 0);
        (vk.lib.vkQueueWaitIdle)(vk.queue);
    }
}

/// Record a layout transition of the whole color image:
/// `layouts` = (old, new), `access` = (src, dst), `stages` = (src, dst).
unsafe fn image_barrier(
    vk: &VulkanCore,
    image: VkImage,
    layouts: (u32, u32),
    access: (VkFlags, VkFlags),
    stages: (VkFlags, VkFlags),
) {
    unsafe {
        #[repr(C)]
        struct ImgBarrier {
            s_type: u32,
            p_next: *const c_void,
            src_access_mask: VkFlags,
            dst_access_mask: VkFlags,
            old_layout: u32,
            new_layout: u32,
            src_queue_family_index: u32,
            dst_queue_family_index: u32,
            image: VkImage,
            subresource_range: [u32; 5],
        }
        let barrier = ImgBarrier {
            s_type: VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER,
            p_next: std::ptr::null(),
            src_access_mask: access.0,
            dst_access_mask: access.1,
            old_layout: layouts.0,
            new_layout: layouts.1,
            src_queue_family_index: u32::MAX,
            dst_queue_family_index: u32::MAX,
            image,
            subresource_range: [1, 0, 1, 0, 1],
        };
        (vk.lib.vkCmdPipelineBarrier)(
            vk.upload_cmd,
            stages.0,
            stages.1,
            0,
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            1,
            &barrier as *const _ as *const c_void,
        );
    }
}
```

- [x] **Step 4: Make sampled textures ready at creation**

Replace `struct TextureInner` with:

```rust
struct TextureInner {
    vk: Option<Arc<VulkanCore>>,
    image: VkImage,
    mem: VkDeviceMemory,
    view: VkImageView,
    /// In `SHADER_READ_ONLY_OPTIMAL` between uploads (every sampled texture).
    ready: bool,
}
```

In `Texture::new`, replace

```rust
                return Self {
                    inner: Arc::new(TextureInner {
                        vk: Some(vk.clone()),
                        image,
                        mem,
                        view,
                    }),
                };
```

with

```rust
                // A sampled texture may be bound before anything is uploaded
                // (the color atlas placeholder): give it a valid layout now.
                let ready = !render_target;
                if ready {
                    begin_upload(vk);
                    image_barrier(
                        vk,
                        image,
                        (VK_IMAGE_LAYOUT_UNDEFINED, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL),
                        (0, VK_ACCESS_SHADER_READ_BIT),
                        (VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT, VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT),
                    );
                    submit_upload(vk);
                }
                return Self {
                    inner: Arc::new(TextureInner {
                        vk: Some(vk.clone()),
                        image,
                        mem,
                        view,
                        ready,
                    }),
                };
```

and in the fallback at the end of `Texture::new` replace

```rust
                vk: None,
                image: 0,
                mem: 0,
                view: 0,
```

with

```rust
                vk: None,
                image: 0,
                mem: 0,
                view: 0,
                ready: false,
```

- [x] **Step 5: Rewrite `write_rect` on the helpers**

Replace the whole `write_rect` method with:

```rust
    #[allow(clippy::too_many_arguments)]
    pub fn write_rect(&self, _queue: &Queue, x: u32, y: u32, width: u32, height: u32, data: &[u8], _bytes_per_row: u32) {
        if width == 0 || height == 0 || data.is_empty() {
            return;
        }
        let Some(vk) = &self.inner.vk else { return };
        unsafe {
            let (stg_buf, stg_mem, stg_ptr) = alloc_host_buffer(
                &vk.lib,
                vk.device,
                &vk.mem_props,
                data.len(),
                VK_BUFFER_USAGE_TRANSFER_SRC_BIT,
            );
            std::ptr::copy_nonoverlapping(data.as_ptr(), stg_ptr, data.len());

            let (old_layout, src_access, src_stage) = upload_source(self.inner.ready);
            begin_upload(vk);
            image_barrier(
                vk,
                self.inner.image,
                (old_layout, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL),
                (src_access, VK_ACCESS_TRANSFER_WRITE_BIT),
                (src_stage, VK_PIPELINE_STAGE_TRANSFER_BIT),
            );

            #[repr(C)]
            struct BufImgCopy {
                buffer_offset: VkDeviceSize,
                buffer_row_length: u32,
                buffer_image_height: u32,
                image_subresource: [u32; 4],
                image_offset: VkOffset3D,
                image_extent: VkExtent3D,
            }
            let region = BufImgCopy {
                buffer_offset: 0,
                buffer_row_length: 0,
                buffer_image_height: 0,
                image_subresource: [1, 0, 0, 1],
                image_offset: VkOffset3D { x: x as i32, y: y as i32, z: 0 },
                image_extent: VkExtent3D { width, height, depth: 1 },
            };
            (vk.lib.vkCmdCopyBufferToImage)(
                vk.upload_cmd,
                stg_buf,
                self.inner.image,
                VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                1,
                &region as *const _ as *const c_void,
            );

            image_barrier(
                vk,
                self.inner.image,
                (VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL),
                (VK_ACCESS_TRANSFER_WRITE_BIT, VK_ACCESS_SHADER_READ_BIT),
                (VK_PIPELINE_STAGE_TRANSFER_BIT, VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT),
            );
            submit_upload(vk);
            (vk.lib.vkDestroyBuffer)(vk.device, stg_buf, std::ptr::null());
            (vk.lib.vkFreeMemory)(vk.device, stg_mem, std::ptr::null());
        }
    }
```

- [x] **Step 6: Run**

Run: `cargo check --tests --target x86_64-unknown-linux-gnu`, `cargo clippy --all-targets --target x86_64-unknown-linux-gnu -- -D warnings` and `cargo test`
Expected: all succeed. On Linux, `cargo test --lib uploads_keep_the_contents_of_a_ready_texture` PASSES.

- [x] **Step 7: Commit checkpoint**

Ready to commit: `src/render/backend/linux_gpu/resources.rs` — `fix(linux): upload textures from their real image layout`

---

## Task 2: Inline images are drawn with their own texture (finding 1)

**Files:**
- Modify: `src/render/backend/linux_gpu/ffi.rs` (one constant; `VkLib` after `vkUpdateDescriptorSets`)
- Modify: `src/render/backend/linux_gpu/device.rs:12-37` (`VulkanCore`), the `VkLib` literal, after the descriptor pool creation (line 462), and the `Self { .. }` literal (line 487)
- Modify: `src/render/backend/linux_gpu/resources.rs` (`TextureInner`, its `Drop`, `Texture::new`, new `Texture::image_dset`)
- Modify: `src/render/backend/linux_gpu/pass.rs:268-304` (`draw_image_quad`, `draw_corners`)
- Test: `src/render/backend/linux_gpu/resources.rs` tests module (created by Task 1)

**Interfaces:**
- Produces in `ffi.rs`: `VK_DESCRIPTOR_POOL_CREATE_FREE_DESCRIPTOR_SET_BIT: u32` and `VkLib::vkFreeDescriptorSets: unsafe extern "C" fn(VkDevice, VkDescriptorPool, u32, *const VkDescriptorSet) -> i32`.
- Produces in `VulkanCore`: `image_desc_pool: VkDescriptorPool`, `image_pool_lock: parking_lot::Mutex<()>`.
- Produces `unsafe fn alloc_image_dset(vk: &VulkanCore, view: VkImageView) -> VkDescriptorSet` (private, `0` on failure) and `pub(super) fn image_dset(&self) -> VkDescriptorSet` on `Texture`.
- Consumes `VulkanCore::{image_dset_layout, image_pipe_layout, image_ubuf, linear_sampler, cells_dsets, cells_pipe_layout}` (existing fields).

- [x] **Step 1: Write the failing test**

Add inside `mod tests` of `resources.rs`:

```rust
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
```

- [x] **Step 2: Confirm the failure**

Run: `cargo check --tests --target x86_64-unknown-linux-gnu`
Expected: FAIL to compile with `no method named image_dset found for struct Texture`.

- [x] **Step 3: Add the pool and the free function**

In `ffi.rs`, next to the other descriptor constants:

```rust
pub(super) const VK_DESCRIPTOR_POOL_CREATE_FREE_DESCRIPTOR_SET_BIT: u32 = 0x0000_0001;
```

and inside `VkLib`, after the `vkUpdateDescriptorSets` field:

```rust
    pub vkFreeDescriptorSets: unsafe extern "C" fn(VkDevice, VkDescriptorPool, u32, *const VkDescriptorSet) -> i32,
```

In `device.rs`, inside the `VkLib { .. }` literal after `vkUpdateDescriptorSets: load_fn!(vkUpdateDescriptorSets),`:

```rust
                vkFreeDescriptorSets: load_fn!(vkFreeDescriptorSets),
```

In `pub struct VulkanCore`, after the `desc_pool` field:

```rust
    /// One set per sampled texture, for the image pipeline; sets are freed
    /// with their texture, so the pool allows it.
    pub(super) image_desc_pool: VkDescriptorPool,
    /// Descriptor pools are not thread safe; guards `image_desc_pool`.
    pub(super) image_pool_lock: parking_lot::Mutex<()>,
```

Above `impl VulkanCore` add:

```rust
/// Sampled textures that can own an image descriptor set at once (glyph
/// atlases plus inline images; a terminal keeps at most 1024 images).
const IMAGE_DESCRIPTOR_SETS: u32 = 2048;
```

In `try_init`, directly after

```rust
            (lib.vkCreateDescriptorPool)(device, &dp_info as *const _ as *const c_void, std::ptr::null(), &mut desc_pool);
```

insert

```rust
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
```

and in the `Some(Arc::new(Self { .. }))` literal, after `desc_pool,`:

```rust
                image_desc_pool,
                image_pool_lock: parking_lot::Mutex::new(()),
```

- [x] **Step 4: Give every sampled texture its set**

In `resources.rs`, above `struct TextureInner`:

```rust
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
```

Add a field to `TextureInner`, after `ready`:

```rust
    /// Descriptor set for drawing this texture as an inline image, or 0.
    image_dset: VkDescriptorSet,
```

In `impl Drop for TextureInner`, directly after `(vk.lib.vkDeviceWaitIdle)(vk.device);` insert:

```rust
                if self.image_dset != 0 {
                    let _pool = vk.image_pool_lock.lock();
                    (vk.lib.vkFreeDescriptorSets)(vk.device, vk.image_desc_pool, 1, &self.image_dset);
                }
```

In `Texture::new`, replace (the block Task 1 wrote)

```rust
                    submit_upload(vk);
                }
                return Self {
                    inner: Arc::new(TextureInner {
                        vk: Some(vk.clone()),
                        image,
                        mem,
                        view,
                        ready,
                    }),
                };
```

with

```rust
                    submit_upload(vk);
                }
                let image_dset = if ready { alloc_image_dset(vk, view) } else { 0 };
                return Self {
                    inner: Arc::new(TextureInner {
                        vk: Some(vk.clone()),
                        image,
                        mem,
                        view,
                        ready,
                        image_dset,
                    }),
                };
```

and in the fallback replace `ready: false,` with:

```rust
                ready: false,
                image_dset: 0,
```

Add to `impl Texture`, after `raw_ptr`:

```rust
    /// Descriptor set binding this texture to the image pipeline; 0 when it
    /// has none (render target, no GPU, or the pool was exhausted).
    #[inline]
    pub(super) fn image_dset(&self) -> VkDescriptorSet {
        self.inner.image_dset
    }
```

- [x] **Step 5: Bind it when drawing**

In `pass.rs`, replace the whole `draw_image_quad` method with:

```rust
    pub fn draw_image_quad<Q: bytemuck::Pod, U: bytemuck::Pod>(
        &mut self,
        pipeline: &ObjPtr,
        quads: &HostBuffer<Q>,
        uniforms: &U,
        texture: &Texture,
        _sampler: SamplerHandle,
        instance_idx: u32,
    ) {
        let Some(vk) = &self.vk else { return };
        let dset = texture.image_dset();
        if pipeline.0 == 0 || dset == 0 {
            return;
        }
        unsafe {
            // Every image of a frame shares these uniforms (target size, flip).
            std::ptr::copy_nonoverlapping(
                uniforms as *const U as *const u8,
                vk.image_ubuf.1,
                std::mem::size_of::<U>(),
            );
            (vk.lib.vkCmdBindPipeline)(vk.upload_cmd, 0, pipeline.0);
            (vk.lib.vkCmdBindDescriptorSets)(vk.upload_cmd, 0, vk.image_pipe_layout, 0, 1, &dset, 0, std::ptr::null());
            let vbuf = quads.raw_ptr();
            let offset = 0u64;
            (vk.lib.vkCmdBindVertexBuffers)(vk.upload_cmd, 0, 1, &vbuf, &offset);
            (vk.lib.vkCmdDraw)(vk.upload_cmd, 4, 1, 0, instance_idx);
        }
    }
```

and replace the whole `draw_corners` method with:

```rust
    pub fn draw_corners<U: bytemuck::Pod>(&mut self, pipeline: &ObjPtr, _uniforms: &U) {
        let Some(vk) = &self.vk else { return };
        if pipeline.0 == 0 {
            return;
        }
        unsafe {
            (vk.lib.vkCmdBindPipeline)(vk.upload_cmd, 0, pipeline.0);
            // An image draw leaves an image-layout set bound; the corner shader
            // reads the cell uniforms the frame's first `draw_cells` wrote.
            let dset = vk.cells_dsets[0];
            (vk.lib.vkCmdBindDescriptorSets)(vk.upload_cmd, 0, vk.cells_pipe_layout, 0, 1, &dset, 0, std::ptr::null());
            (vk.lib.vkCmdDraw)(vk.upload_cmd, 4, 1, 0, 0);
        }
    }
```

- [x] **Step 6: Run**

Run: `cargo check --tests --target x86_64-unknown-linux-gnu`, `cargo clippy --all-targets --target x86_64-unknown-linux-gnu -- -D warnings` and `cargo test`
Expected: all succeed. On Linux, `cargo test --lib sampled_textures_own_an_image_descriptor_set` PASSES.

The draw itself has no automated test (it needs a presented frame); it is covered by items 2 to 4 of Task 4's manual checklist.

- [x] **Step 7: Commit checkpoint**

Ready to commit: `src/render/backend/linux_gpu/ffi.rs`, `src/render/backend/linux_gpu/device.rs`, `src/render/backend/linux_gpu/resources.rs`, `src/render/backend/linux_gpu/pass.rs` — `fix(linux): bind each inline image's texture when drawing it`

---

## Task 3: Small uploads reuse one staging buffer (finding 9, cost)

**Files:**
- Modify: `src/render/backend/linux_gpu/resources.rs` (new `Staging`, `staging_capacity`; `Texture::new`; `Texture::write_rect`)
- Modify: `src/render/backend/linux_gpu/device.rs` (`VulkanCore` field and the `Self { .. }` literal)
- Test: `src/render/backend/linux_gpu/resources.rs` tests module

**Interfaces:**
- Produces `pub(super) struct Staging { buf: VkBuffer, mem: VkDeviceMemory, ptr: *mut u8, cap: usize }` with `pub(super) const fn empty() -> Self`, and `fn staging_capacity(len: usize) -> Option<usize>`.
- Produces in `VulkanCore`: `staging: parking_lot::Mutex<super::resources::Staging>`; holding it also serialises every use of `upload_cmd` outside a frame.

- [x] **Step 1: Write the failing test**

Add inside `mod tests` of `resources.rs`:

```rust
    #[test]
    fn staging_capacity_is_bounded_and_grows_by_powers_of_two() {
        assert_eq!(staging_capacity(1), Some(64 << 10), "a glyph upload uses the minimum buffer");
        assert_eq!(staging_capacity(64 << 10), Some(64 << 10));
        assert_eq!(staging_capacity((64 << 10) + 1), Some(128 << 10));
        assert_eq!(staging_capacity(4 << 20), Some(4 << 20));
        // A large inline image must not pin its staging memory while idle.
        assert_eq!(staging_capacity((4 << 20) + 1), None);
    }
```

- [x] **Step 2: Confirm the failure**

Run: `cargo check --tests --target x86_64-unknown-linux-gnu`
Expected: FAIL to compile with `cannot find function staging_capacity in this scope`.

- [x] **Step 3: Add the staging buffer**

In `resources.rs`, above `upload_source`:

```rust
/// Smallest shared staging buffer: a few hundred glyph cells.
const STAGING_MIN: usize = 64 << 10;
/// Largest staging buffer kept between uploads; bigger uploads (inline
/// images) get a one-off buffer so idle memory does not hold their size.
const STAGING_KEEP_MAX: usize = 4 << 20;

/// Capacity the shared staging buffer needs for a `len`-byte upload, or
/// `None` when the upload is too large to be kept around.
fn staging_capacity(len: usize) -> Option<usize> {
    (len <= STAGING_KEEP_MAX).then(|| len.next_power_of_two().max(STAGING_MIN))
}

/// Upload staging memory shared by every texture of the device.
pub(super) struct Staging {
    buf: VkBuffer,
    mem: VkDeviceMemory,
    ptr: *mut u8,
    cap: usize,
}

impl Staging {
    pub(super) const fn empty() -> Self {
        Self { buf: 0, mem: 0, ptr: std::ptr::null_mut(), cap: 0 }
    }
}
```

In `device.rs`, add to `pub struct VulkanCore` after `image_pool_lock`:

```rust
    /// Shared upload staging buffer. Its lock is held for a whole upload, so
    /// it also serialises every use of `upload_cmd` outside a frame.
    pub(super) staging: parking_lot::Mutex<super::resources::Staging>,
```

and to the `Some(Arc::new(Self { .. }))` literal after `image_pool_lock: parking_lot::Mutex::new(()),`:

```rust
                staging: parking_lot::Mutex::new(super::resources::Staging::empty()),
```

- [x] **Step 4: Use it**

In `Texture::new`, replace

```rust
                if ready {
                    begin_upload(vk);
```

with

```rust
                if ready {
                    let _upload = vk.staging.lock();
                    begin_upload(vk);
```

In `Texture::write_rect`, replace

```rust
            let (stg_buf, stg_mem, stg_ptr) = alloc_host_buffer(
                &vk.lib,
                vk.device,
                &vk.mem_props,
                data.len(),
                VK_BUFFER_USAGE_TRANSFER_SRC_BIT,
            );
            std::ptr::copy_nonoverlapping(data.as_ptr(), stg_ptr, data.len());
```

with

```rust
            // Held until the upload finished: the previous one is complete
            // (`submit_upload` waits), so the buffer is free to reuse or replace.
            let mut staging = vk.staging.lock();
            let one_off = match staging_capacity(data.len()) {
                Some(cap) => {
                    if staging.cap < cap {
                        if staging.buf != 0 {
                            (vk.lib.vkDestroyBuffer)(vk.device, staging.buf, std::ptr::null());
                            (vk.lib.vkFreeMemory)(vk.device, staging.mem, std::ptr::null());
                        }
                        let (buf, mem, ptr) =
                            alloc_host_buffer(&vk.lib, vk.device, &vk.mem_props, cap, VK_BUFFER_USAGE_TRANSFER_SRC_BIT);
                        *staging = Staging { buf, mem, ptr, cap };
                    }
                    None
                }
                None => Some(alloc_host_buffer(
                    &vk.lib,
                    vk.device,
                    &vk.mem_props,
                    data.len(),
                    VK_BUFFER_USAGE_TRANSFER_SRC_BIT,
                )),
            };
            let (stg_buf, stg_ptr) = match one_off {
                Some((buf, _, ptr)) => (buf, ptr),
                None => (staging.buf, staging.ptr),
            };
            std::ptr::copy_nonoverlapping(data.as_ptr(), stg_ptr, data.len());
```

and replace

```rust
            submit_upload(vk);
            (vk.lib.vkDestroyBuffer)(vk.device, stg_buf, std::ptr::null());
            (vk.lib.vkFreeMemory)(vk.device, stg_mem, std::ptr::null());
```

with

```rust
            submit_upload(vk);
            if let Some((buf, mem, _)) = one_off {
                (vk.lib.vkDestroyBuffer)(vk.device, buf, std::ptr::null());
                (vk.lib.vkFreeMemory)(vk.device, mem, std::ptr::null());
            }
```

- [x] **Step 5: Run**

Run: `cargo check --tests --target x86_64-unknown-linux-gnu`, `cargo clippy --all-targets --target x86_64-unknown-linux-gnu -- -D warnings` and `cargo test`
Expected: all succeed. On Linux, `cargo test --lib staging_capacity` PASSES.

- [x] **Step 6: Commit checkpoint**

Ready to commit: `src/render/backend/linux_gpu/resources.rs`, `src/render/backend/linux_gpu/device.rs` — `perf(linux): reuse one staging buffer for small texture uploads`

---

## Task 4: Verification & Docs

**Files:**
- Modify: `docs/reviews/2026-11-11-linux-history-ai-graphics.md` (status notes)

- [x] **Step 1: Mark the fixed findings in the review**

Add this line directly under each of the headings `### 1.` and `### 9.`:

```markdown
**Status:** fixed by [plan C](../plans/2026-11-11-review-fixes-c-vulkan-images.md); Linux run pending user verification. Batching a frame's glyph uploads into one submission remains a roadmap follow-up.
```

`README.md` needs no change: inline images are already listed as a feature and no key or binding changes. The upload-batching follow-up is recorded in `ROADMAP.md` by plan A, Task 5.

- [x] **Step 2: Read the full diff**

Run: `git status --short` and `git diff`
Expected: only `src/render/backend/linux_gpu/{ffi,device,resources,pass}.rs` and the review file.

- [x] **Step 3: Full verification (macOS)**

Run each and quote the result:

```bash
cargo check
cargo check --target x86_64-unknown-linux-gnu
cargo check --tests --target x86_64-unknown-linux-gnu
cargo test
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --target x86_64-unknown-linux-gnu -- -D warnings
git diff --check
```

Expected: all succeed, zero warnings. If a Linux-target command fails for a reason unrelated to this branch (it also fails on `main`), report that with the output instead of working around it.

- [ ] **Step 4: Manual checklist for the user (Linux, x86_64)** _(2026-11-12: pending on real hardware. Run on macOS in an x86_64 Debian container (qemu, lavapipe, Xvfb, validation layers), binaries cross-built with `cargo zigbuild`: `linux_gpu` tests 11 passed with a real lavapipe device. Item 2 passed with before/after evidence: a 64x64 magenta Kitty PNG gives 0 magenta pixels on `main` (with `VUID-vkCmdDraw-None-08600`) and 4,096 on the branch, without that error. Item 5 passed (six window open/close cycles, image drawn again). Text stayed drawn across resizes; items 1 (glyph stress), 3, 4, 6 and 7 as written were not run.)_

Hand this list to the user and record each answer; anything not run is reported as unverified.

```bash
cargo test --lib linux_gpu
```

Expected: `uploads_keep_the_contents_of_a_ready_texture`, `sampled_textures_own_an_image_descriptor_set` and `staging_capacity_is_bounded_and_grows_by_powers_of_two` pass, with the tests of plan B. These tests share one Vulkan device and its command buffer with the renderer tests; if a failure looks like a race, rerun with `cargo test --lib linux_gpu -- --test-threads=1` and report both results.

1. Text: start the app, run `ls -la /usr/bin`, scroll, then print a few hundred new glyphs (`python3 -c 'print("".join(chr(c) for c in range(0x400, 0x600)))'`). Every glyph printed earlier is still drawn correctly.
2. Kitty image: `kitty +kitten icat some.png` (or `printf` a small `a=T,f=100` transmission). The picture shows the image, not glyphs or noise.
3. Sixel: `img2sixel some.png`. Same expectation.
4. Image under text: an image placed with a negative z (`icat --z-index=-1`) shows the text on top; the window corners stay rounded when `window-corner-radius` and a translucent background are set.
5. Close and reopen the window, repeat item 2: no crash, no `litty:` error.
6. Idle memory: with one window idle after items 1 to 3, `grep VmRSS /proc/$(pgrep -n litty)/status`; report it next to the same reading on `main`.
7. Optional, if the validation layers are installed: `VK_INSTANCE_LAYERS=VK_LAYER_KHRONOS_validation ./target/release/litty 2> vk.log`, repeat items 1 and 2, and attach any `VUID` line mentioning image layout or descriptor sets.

- [x] **Step 5: Self-review and independent review** _(2026-11-12: `/code-review medium` on the combined diff of plans B and C; no finding required a code change, see the PR description)_

Go through the nine-item self-review checklist of the user's global rules with a one-line reason per item, then request an independent review of the diff (`/code-review` or a review subagent given the diff and this plan's Confirmed Requirements). Address or report its findings; if anything changed, repeat from Step 2.

- [x] **Step 6: Commit checkpoint**

Ready to commit: `docs/reviews/2026-11-11-linux-history-ai-graphics.md` — `docs: mark vulkan image findings as fixed`

**Verification gaps to report:** nothing in this plan runs on the macOS
development machine beyond compiling for the Linux target. Layout
transitions, descriptor binding and the drawn result are verified only by the
user's manual checklist; until it is returned, the branch is "compiles, not
verified on Linux". Item 7 depends on the validation layers being installed.
