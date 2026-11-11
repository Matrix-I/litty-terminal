# Review fixes B (Vulkan robustness) Implementation Plan — 2026-11-11

> **For agentic workers:** REQUIRED SUB-SKILL: development-superpowers:subagent-driven-development or executing-plans-strict. Steps use `- [ ]`.

**Goal:** Fix findings 3, 4, 5 and 8 of
[the 2026-11-11 review](../reviews/2026-11-11-linux-history-ai-graphics.md):
the Linux Vulkan backend must not use a destroyed swapchain, must not run
custom shaders with fake handles, must initialise when only some window-system
extensions exist, and must release a window's GPU objects when it closes.
Findings 2, 6, 7, 10 are in [plan A](./2026-11-11-review-fixes-a-graphics-history-ai.md);
findings 1 and 9 are in [plan C](./2026-11-11-review-fixes-c-vulkan-images.md).

**Architecture:** Four changes inside `src/render/backend/linux_gpu/`, one
commit each. Decisions are extracted into pure functions (`swapchain_extent`,
`instance_extensions`, `graphics_family`) so they have unit tests; the Vulkan
calls around them are verified by hand on a Linux machine.

**Tech Stack:** Rust 2024, Vulkan 1.0 through `dlopen`, no new dependency.

**Branch:** `fix/vulkan-robustness` (from `main`).

## Confirmed Requirements

1. `custom-shader` on Linux is reported as unsupported: the pipeline constructor returns an error, a warning is printed, and the terminal runs without the shader. Real support is a roadmap item. (user, 2026-11-11)
2. Linux behaviour is verified by the user on a Linux machine. Tasks contain unit tests for pure logic, a compile check for the Linux target, and a manual checklist. (user, 2026-11-11)
3. The ten main findings are fixed in three independent plans, one branch each; this is plan B. (user, 2026-11-11)

## Global Constraints

From `CLAUDE.md` (project):

- Performance: 0% idle CPU, under 50 MB idle RAM with all windows closed, zero heap allocation on hot paths (render loop). Task 2's `swapchain_extent` and Task 3's `Drop` run only on reconfigure and close.
- Platform code: every platform-specific path needs a counterpart on the other platform so both targets compile. All four tasks change Linux-only files; the macOS backend already implements `create_postfx_pipeline`, surface teardown and device selection.
- Bug fixes start with a failing regression test. Where a test needs a live Vulkan device (Tasks 2 and 3, partly), the plan says so and gives a scripted manual check instead.
- Scope: change only what the task needs.
- Verification before every commit that touches `src/`:
  - `cargo check`
  - `cargo check --target x86_64-unknown-linux-gnu`
  - `cargo test` (outside the sandbox; PTY tests require `openpty`)
  - `cargo clippy --all-targets -- -D warnings`
  - `git diff --check`
- Commit only when asked; never push. Conventional Commits `type(scope): summary`, lowercase, imperative, no trailing period, at most 72 characters. Stage files explicitly. No `Co-Authored-By` trailer.
- Update `README.md` / docs when user-visible behaviour or config keys change.

Supported Linux target: x86_64 (including Arch Linux).

**The tests in `linux_gpu/` compile and run only on Linux.** On macOS the
executor proves they compile with
`cargo check --tests --target x86_64-unknown-linux-gnu` and reports them as
"not run here"; the user runs `cargo test --lib linux_gpu` on Linux.

## Technical Defaults

- `swapchain_extent` returns the surface's `current_extent` when it is fixed (X11) and the clamped request when it is `0xFFFFFFFF` (Wayland convention) — this is what the Vulkan specification requires of `imageExtent`.
- A zero-area surface (minimised, or capabilities query failed) gets no swapchain; `acquire_frame` already returns `None` for `swapchain == 0`.
- The first physical device with a graphics queue family is used. Choosing by present support or device type needs a surface before device creation and is a roadmap follow-up (recorded by plan A).
- Vulkan initialisation failures print one line to stderr with the `litty:` prefix, as the X11 code does.

## Risks & Rollback

| Risk | Mitigation | Revert |
|---|---|---|
| Task 1 changes user-visible behaviour: a Linux config with `custom-shader` now prints a warning instead of crashing | Confirmed requirement 1; documented in `docs/guide/configuration.md` | Revert the Task 1 commit |
| Task 2: a wrong extent rule leaves a window without a swapchain (black) | Unit tests for the three surface kinds; manual resize/minimise checks | Revert the Task 2 commit |
| Task 3: destroying the surface after the X11 display is closed would crash | `State` declares `renderer` before `window` (`src/app/state.rs:19-22`), so the surface drops first; manual close test | Revert the Task 3 commit |
| Task 4: stricter init rejects a setup that worked by accident (no `VK_KHR_surface`) | Such a setup could never present; the failure is now logged | Revert the Task 4 commit |
| None of this can run on the macOS development machine | Compile check for the Linux target plus the manual checklist in Task 5; every unverified item is reported | — |

No persisted data, config format, keybinding or public API changes.

---

## Task 1: `custom-shader` is rejected on Linux (finding 4)

**Files:**
- Modify: `src/render/backend/linux_gpu/device.rs:1044-1047` (`Device::create_postfx_pipeline`)
- Test: `src/render/backend/linux_gpu/device.rs` (new `tests` module at the end of the file)
- Modify: `docs/guide/configuration.md:57`

**Interfaces:** `pub fn create_postfx_pipeline(&self, _msl_src: &str, _format: TextureFormat) -> Result<ObjPtr, String>` keeps its signature. Its only caller, `PostFx::new` (`src/render/postfx.rs:315`), already prints the error and returns `None` when no pipeline was built.

- [ ] **Step 1: Write the failing test**

Append to `src/render/backend/linux_gpu/device.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_shaders_are_rejected() {
        let device = Device { vk: None };
        let err = device.create_postfx_pipeline("void mainImage() {}", TextureFormat::Bgra8Unorm).err();
        assert_eq!(err.as_deref(), Some("custom shaders are not supported on Linux yet"));
    }
}
```

- [ ] **Step 2: Confirm it compiles and would fail**

Run: `cargo check --tests --target x86_64-unknown-linux-gnu`
Expected: compiles. On Linux, `cargo test --lib custom_shaders_are_rejected` FAILS with `left: None` (the function returns `Ok(ObjPtr(4))`).

- [ ] **Step 3: Fix**

Replace the body of `create_postfx_pipeline`:

```rust
    /// Custom shaders need GLSL to SPIR-V translation and offscreen targets,
    /// which this backend does not have. Returning a handle here made the
    /// renderer bind it as a real pipeline.
    pub fn create_postfx_pipeline(&self, _msl_src: &str, _format: TextureFormat) -> Result<ObjPtr, String> {
        Err("custom shaders are not supported on Linux yet".into())
    }
```

- [ ] **Step 4: Document it**

In `docs/guide/configuration.md`, replace line 57

```markdown
| **Shaders & Effects** | `custom-shader` | Path to GLSL file (repeatable) | None | Shadertoy GLSL post-processing shader pass (chains in file order) |
```

with

```markdown
| **Shaders & Effects** | `custom-shader` | Path to GLSL file (repeatable) | None | Shadertoy GLSL post-processing shader pass (chains in file order). macOS only: on Linux the key is ignored with a warning |
```

- [ ] **Step 5: Run**

Run: `cargo check --tests --target x86_64-unknown-linux-gnu` and `cargo test`
Expected: both succeed (the new test is not compiled on macOS). On Linux, `cargo test --lib custom_shaders_are_rejected` PASSES.

- [ ] **Step 6: Commit checkpoint**

Ready to commit: `src/render/backend/linux_gpu/device.rs`, `docs/guide/configuration.md` — `fix(linux): reject custom shaders instead of using fake handles`

---

## Task 2: Swapchain creation cannot leave a destroyed handle (finding 3)

**Files:**
- Modify: `src/render/backend/linux_gpu/surface.rs:180-271` (`Surface::configure`)
- Test: `src/render/backend/linux_gpu/surface.rs` tests module (line 401)

**Interfaces:** Produces `fn swapchain_extent(caps: &VkSurfaceCapabilitiesKHR, width: u32, height: u32) -> Option<VkExtent2D>` (private). Consumes `VkSurfaceCapabilitiesKHR` and `VkExtent2D` from `ffi.rs` (both `Copy + Default`).

- [ ] **Step 1: Write the failing tests**

Add to the tests module of `surface.rs`:

```rust
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
```

- [ ] **Step 2: Confirm the failure**

Run: `cargo check --tests --target x86_64-unknown-linux-gnu`
Expected: FAIL to compile with `cannot find function swapchain_extent in this scope`.

- [ ] **Step 3: Implement `swapchain_extent`**

Add to `surface.rs` above `pub struct Surface`:

```rust
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
```

- [ ] **Step 4: Use it in `configure` and fix the failure path**

In `Surface::configure`, directly after

```rust
            let mut caps = VkSurfaceCapabilitiesKHR::default();
            (vk.lib.vkGetPhysicalDeviceSurfaceCapabilitiesKHR)(vk.phys, self.surface_khr, &mut caps);
```

insert

```rust
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
```

In the `SwapInfo` initialiser replace

```rust
                image_extent: VkExtent2D { width: self.width, height: self.height },
```

with

```rust
                image_extent: extent,
```

Replace

```rust
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
```

with

```rust
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
```

- [ ] **Step 5: Run**

Run: `cargo check --tests --target x86_64-unknown-linux-gnu`, `cargo clippy --all-targets --target x86_64-unknown-linux-gnu -- -D warnings` and `cargo test`
Expected: all succeed. On Linux, `cargo test --lib swapchain_extent` PASSES (3 tests) and `framebuffer_extent_tracks_register_and_forget` still passes.

The failure path itself (`vkCreateSwapchainKHR` returning an error) has no automated test: it needs a live device and a surface in a state the driver rejects. It is covered by the resize and minimise items of Task 5's manual checklist.

- [ ] **Step 6: Commit checkpoint**

Ready to commit: `src/render/backend/linux_gpu/surface.rs` — `fix(linux): never keep a destroyed swapchain after a failed create`

---

## Task 3: A closed window releases its GPU objects (finding 8)

**Files:**
- Modify: `src/render/backend/linux_gpu/ffi.rs` (`VkLib`, after `vkResetFences` and after `vkQueuePresentKHR`)
- Modify: `src/render/backend/linux_gpu/device.rs:243-306` (the `VkLib { .. }` literal)
- Modify: `src/render/backend/linux_gpu/surface.rs:380-398` (`shrink_drawables`) and a new `impl Drop for Surface`

**Interfaces:**
- Produces in `VkLib`: `vkDestroySemaphore: unsafe extern "C" fn(VkDevice, VkSemaphore, *const c_void)`, `vkDestroyFence: unsafe extern "C" fn(VkDevice, VkFence, *const c_void)`, `vkDestroySurfaceKHR: unsafe extern "C" fn(VkInstance, VkSurfaceKHR, *const c_void)`.
- Produces `fn release_swapchain(&mut self)` on `Surface` (private); `pub fn shrink_drawables(&mut self, _device: &Device, _queue: &Queue)` keeps its signature.

No automated test is possible: every object involved exists only with a live Vulkan device and window, and the tests of this module run with `vk: None` on machines without a GPU. The regression check is the scripted manual repro in Step 5.

- [ ] **Step 1: Record the current behaviour (manual, Linux)**

On the Linux machine, on `main`:

```bash
cargo build --release && ./target/release/litty &
```

Open and close ten extra windows (`Ctrl+Shift+N`, then `Ctrl+Shift+W` on the only tab of each new window, which closes it), leaving one open, then read `grep VmRSS /proc/$(pgrep -n litty)/status`. Note the value as "before". Expected on `main`: RSS grows with every closed window.

- [ ] **Step 2: Load the destroy functions**

In `ffi.rs`, inside `pub(super) struct VkLib`, add after the `vkResetFences` field:

```rust
    pub vkDestroySemaphore: unsafe extern "C" fn(VkDevice, VkSemaphore, *const c_void),
    pub vkDestroyFence: unsafe extern "C" fn(VkDevice, VkFence, *const c_void),
```

and after the `vkQueuePresentKHR` field:

```rust
    pub vkDestroySurfaceKHR: unsafe extern "C" fn(VkInstance, VkSurfaceKHR, *const c_void),
```

In `device.rs`, inside the `let lib = VkLib { .. }` literal, add after `vkResetFences: load_fn!(vkResetFences),`:

```rust
                vkDestroySemaphore: load_fn!(vkDestroySemaphore),
                vkDestroyFence: load_fn!(vkDestroyFence),
```

and after `vkQueuePresentKHR: load_fn!(vkQueuePresentKHR),`:

```rust
                vkDestroySurfaceKHR: load_fn!(vkDestroySurfaceKHR),
```

- [ ] **Step 3: Share the teardown and add `Drop`**

In `surface.rs`, replace the whole `shrink_drawables` method with:

```rust
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
```

and add after the `impl Surface { .. }` block:

```rust
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
```

- [ ] **Step 4: Compile**

Run: `cargo check --tests --target x86_64-unknown-linux-gnu`, `cargo clippy --all-targets --target x86_64-unknown-linux-gnu -- -D warnings` and `cargo test`
Expected: all succeed.

- [ ] **Step 5: Repeat the manual repro (Linux)**

Repeat Step 1 on the branch. Expected: RSS after ten open/close cycles stays close to the single-window value (report both numbers), and closing a window prints no `litty: X11 error` line.

- [ ] **Step 6: Commit checkpoint**

Ready to commit: `src/render/backend/linux_gpu/ffi.rs`, `src/render/backend/linux_gpu/device.rs`, `src/render/backend/linux_gpu/surface.rs` — `fix(linux): release swapchain and surface when a window closes`

---

## Task 4: Vulkan starts with the extensions and queue family that exist (finding 5)

**Files:**
- Modify: `src/render/backend/linux_gpu/ffi.rs` (two structs and one constant, after `VkSurfaceCapabilitiesKHR`)
- Modify: `src/render/backend/linux_gpu/device.rs:116-241` (`VulkanCore::try_init`) and `:308-323`
- Test: `src/render/backend/linux_gpu/device.rs` tests module (created by Task 1, so Task 1 must be done first)

**Interfaces:**
- Produces in `ffi.rs`: `VkExtensionProperties { extension_name: [c_char; 256], spec_version: u32 }`, `VkQueueFamilyProperties { queue_flags: VkFlags, queue_count: u32, timestamp_valid_bits: u32, min_image_transfer_granularity: VkExtent3D }`, `VK_QUEUE_GRAPHICS_BIT: u32`.
- Produces in `device.rs`: `fn instance_extensions(available: &[&CStr]) -> Option<Vec<&'static CStr>>` and `fn graphics_family(families: &[VkQueueFamilyProperties]) -> Option<u32>` (both private).

- [ ] **Step 1: Write the failing tests**

Add inside `mod tests` of `device.rs`:

```rust
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
```

- [ ] **Step 2: Confirm the failure**

Run: `cargo check --tests --target x86_64-unknown-linux-gnu`
Expected: FAIL to compile: `instance_extensions`, `graphics_family`, `VkQueueFamilyProperties` and `VK_QUEUE_GRAPHICS_BIT` do not exist.

- [ ] **Step 3: Add the FFI types**

In `ffi.rs`, after the `VkSurfaceCapabilitiesKHR` struct:

```rust
pub(super) const VK_QUEUE_GRAPHICS_BIT: u32 = 0x0000_0001;

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
```

- [ ] **Step 4: Add the two decisions**

In `device.rs`, above `impl VulkanCore`:

```rust
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
```

- [ ] **Step 5: Use them in `try_init`**

In `VulkanCore::try_init`, replace the block from `let exts_ws = [` through the closing brace of the `if vkCreateInstance(..) != VK_SUCCESS { .. }` retry (lines 116-139) with:

```rust
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
```

Replace

```rust
            let phys = pds[0];
```

with

```rust
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
```

In the `QueueInfo` initialiser (`let q_info = QueueInfo { .. }`) replace `queue_family_index: 0,` with `queue_family_index: queue_family,`.

Replace

```rust
            (lib.vkGetDeviceQueue)(device, 0, 0, &mut queue);
```

with

```rust
            (lib.vkGetDeviceQueue)(device, queue_family, 0, &mut queue);
```

In the `PoolInfo` initialiser (`let pool_info = PoolInfo { .. }`) replace `queue_family_index: 0,` with `queue_family_index: queue_family,`.

No import is needed for `VkExtensionProperties`, `VkQueueFamilyProperties` and `VK_QUEUE_GRAPHICS_BIT`: `device.rs` already has `use super::ffi::*;`.

- [ ] **Step 6: Run**

Run: `cargo check --tests --target x86_64-unknown-linux-gnu`, `cargo clippy --all-targets --target x86_64-unknown-linux-gnu -- -D warnings` and `cargo test`
Expected: all succeed. On Linux, `cargo test --lib instance_extensions` and `cargo test --lib graphics_family` PASS (3 tests).

- [ ] **Step 7: Commit checkpoint**

Ready to commit: `src/render/backend/linux_gpu/ffi.rs`, `src/render/backend/linux_gpu/device.rs` — `fix(linux): request only available vulkan extensions and queues`

---

## Task 5: Verification & Docs

**Files:**
- Modify: `docs/reviews/2026-11-11-linux-history-ai-graphics.md` (status notes)

- [ ] **Step 1: Mark the fixed findings in the review**

Add this line directly under each of the headings `### 3.`, `### 4.`, `### 5.` and `### 8.`:

```markdown
**Status:** fixed by [plan B](../plans/2026-11-11-review-fixes-b-vulkan-robustness.md); Linux run pending user verification.
```

`README.md` needs no change: `custom-shader` is documented only in `docs/guide/configuration.md` (updated by Task 1). `ROADMAP.md` follow-ups for custom shaders and device selection are added by plan A, Task 5.

- [ ] **Step 2: Read the full diff**

Run: `git status --short` and `git diff`
Expected: only `src/render/backend/linux_gpu/{ffi,device,surface}.rs`, `docs/guide/configuration.md` and the review file.

- [ ] **Step 3: Full verification (macOS)**

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

- [ ] **Step 4: Manual checklist for the user (Linux, x86_64)**

Hand this list to the user and record each answer; anything not run is reported as unverified.

```bash
cargo test --lib linux_gpu
```

Expected: `custom_shaders_are_rejected`, three `swapchain_extent_*`, two `instance_extensions_*`, `graphics_family_skips_families_that_cannot_draw` and `framebuffer_extent_tracks_register_and_forget` pass.

1. Start the app: a window with text appears, no `litty:` error on stderr.
2. Resize the window quickly for ten seconds, maximise and restore: the content follows, no crash, no `could not create a ... swapchain` line (if it appears, the window must recover on the next resize).
3. Minimise and restore: the window redraws.
4. Add `custom-shader = /tmp/x.glsl` (any existing GLSL file) to the config and restart: one `litty: custom-shader compile error: custom shaders are not supported on Linux yet` line, and the terminal works normally.
5. Task 3, Step 5: RSS before and after ten open/close cycles.
6. `VK_ICD_FILENAMES` or `vulkaninfo --summary` shows which device is used on a hybrid-GPU machine, if one is available.

- [ ] **Step 5: Self-review and independent review**

Go through the nine-item self-review checklist of the user's global rules with a one-line reason per item, then request an independent review of the diff (`/code-review` or a review subagent given the diff and this plan's Confirmed Requirements). Address or report its findings; if anything changed, repeat from Step 2.

- [ ] **Step 6: Commit checkpoint**

Ready to commit: `docs/reviews/2026-11-11-linux-history-ai-graphics.md` — `docs: mark vulkan robustness findings as fixed`

**Verification gaps to report:** nothing in this plan runs on the macOS
development machine beyond compiling for the Linux target. The swapchain
failure path, `Drop for Surface` and instance creation are verified only by
the user's manual checklist; until it is returned, the branch is "compiles,
not verified on Linux".
