# Code review: Vulkan/Linux backend, history, AI, graphics (2026-11-11)

Scope, at commit `ac5a861` on `main`:

- `src/render/backend/linux_gpu/`, `src/render/atlas/linux.rs`, `src/platform/linux/`
- `src/terminal/history.rs`, `src/app/history_ui.rs`
- `src/ai/`
- `src/terminal/graphics/`

**Method and limits.** Read-through only, on macOS. Nothing was compiled for
Linux, run, or reproduced, so every finding below is unverified against a real
Linux machine. Each one needs a failing regression test (or a scripted repro
where a test is impossible) before it is fixed.

**Not read:** `platform/linux/native/events.rs`, most of `linux_gpu/ffi.rs`
(only the struct layouts), `ai/prompt.rs`, `ai/live_tests.rs`,
`graphics/tests.rs`, `render/renderer/ui/history_popup.rs`.

## Summary

| Area | Findings | Worst case |
|---|---|---|
| Vulkan/Linux backend | 6 main + 5 secondary | Driver crash, images not rendered |
| `graphics/` | 1 main + 4 secondary | Wrong animation timing (debug-only panic) |
| `history.rs` | 2 main + 2 secondary | Deleted entry stays on disk |
| `ai/` | 1 main + 2 secondary | ~4 minute wait on a permanent error |

## Main findings (most severe first)

### 1. Inline images never bind their texture on Vulkan

- **Where:** `src/render/backend/linux_gpu/pass.rs:268` (`draw_image_quad`)
- **Defect:** `_texture` and `_sampler` are ignored. No descriptor set is ever
  allocated for `image_pipe_layout`, and `image_ubuf` is written but attached
  to no set. `image.wgsl` expects a uniform, a texture and a sampler in group 0.
- **Failure:** `kitty +icat img.png` or Sixel output on Linux draws with the
  cells descriptor set, whose layout is incompatible (the glyph atlas is
  sampled instead of the image). For images with `z < Z_BELOW_BG`, drawn before
  any `draw_cells`, no set is bound at all. Result: garbage quad or a driver
  crash.
- **Fix direction:** one descriptor set per image texture (or a small pool
  rewritten per draw), bound with `image_pipe_layout`, with `image_ubuf` at
  binding 0.

### 2. Animation loop length overflows (`cycle * plays`) — corrected

**Status:** fixed by [plan A](../plans/2026-11-11-review-fixes-a-graphics-history-ai.md).

> **Correction (2026-11-11).** This finding first claimed that
> `now - Duration::from_millis(..)` at `animation.rs:253` panics. That was
> wrong: checked on macOS with rustc 1.99.1, `Instant::now() - 13.6 years`
> succeeds, because the Unix `Instant` may go below its origin. It was never a
> crash from program output. What remains is the smaller defect below.

- **Where:** `src/terminal/graphics/animation.rs:98` (`Image::frame_at`)
- **Defect:** `cycle * plays as u64` overflows when frame gaps (`z=`, up to
  `i32::MAX` ms each, 256 frames) and the loop count (`v=`, up to `u32::MAX`)
  are both large.
- **Failure:** a panic in debug builds only. Release builds have no overflow
  checks, so the product wraps and the animation stops at an arbitrary time.
  Severity: low.
- **Fix direction:** `cycle.saturating_mul(plays as u64)`.

### 3. Failed swapchain creation leaves a destroyed handle in use

**Status:** fixed by [plan B](../plans/2026-11-11-review-fixes-b-vulkan-robustness.md); Linux run pending user verification.

- **Where:** `src/render/backend/linux_gpu/surface.rs:263`
- **Defect:** when `vkCreateSwapchainKHR` fails, `self.swapchain` still equals
  `old_sc`, which is destroyed on the next line. The `== 0` guard does not
  trigger, so the dead handle goes to `vkGetSwapchainImagesKHR` and later to
  `acquire_frame`. The requested extent is also never clamped to
  `caps.min_image_extent` / `max_image_extent` / `current_extent`, which makes
  the failure reachable on X11 during a resize.
- **Failure:** use-after-destroy in the driver: crash or a permanently black
  window.
- **Fix direction:** set `self.swapchain = 0` on failure; clamp the extent.

### 4. `custom-shader` on Linux runs with fake Vulkan handles

**Status:** fixed by [plan B](../plans/2026-11-11-review-fixes-b-vulkan-robustness.md); Linux run pending user verification.

- **Where:** `src/render/backend/linux_gpu/device.rs:1045`
  (`create_postfx_pipeline`), used by `src/render/postfx.rs`
- **Defect:** the function returns `Ok(ObjPtr(4))`, so `PostFx` is enabled on
  Linux. The cell pass then gets `Texture::raw_ptr()` (a `VkImageView`) as the
  framebuffer for `vkCmdBeginRenderPass`, and `draw_postfx` binds pipeline
  handle `4`.
- **Failure:** setting `custom-shader = foo.glsl` on Linux crashes in the
  driver on the first frame.
- **Fix direction:** return `Err("custom shaders are not supported on Linux
  yet")` until the path is implemented, and say so in `README.md`.

### 5. Instance creation requires all three WSI extensions at once

**Status:** fixed by [plan B](../plans/2026-11-11-review-fixes-b-vulkan-robustness.md); Linux run pending user verification.

- **Where:** `src/render/backend/linux_gpu/device.rs:116`
- **Defect:** `VK_KHR_wayland_surface`, `VK_KHR_xlib_surface` and
  `VK_KHR_xcb_surface` are requested together. If one is missing, the retry
  uses zero extensions, after which the mandatory surface and swapchain symbols
  are null and `try_init` returns `None`. Separately, the code takes `pds[0]`
  and queue family `0` without checking graphics or present support
  (`vkGetPhysicalDeviceSurfaceSupportKHR` is never called).
- **Failure:** on a Mesa build without one platform, Vulkan init fails silently
  and the window renders nothing. On hybrid-GPU laptops `pds[0]` may be a
  device that cannot present to the surface.
- **Fix direction:** enumerate instance extensions and enable only the
  available ones; pick a device and queue family that support graphics and
  present for the surface; report a clear error when none exists.

### 6. Fish history unescape order corrupts an escaped backslash before `n`

**Status:** fixed by [plan A](../plans/2026-11-11-review-fixes-a-graphics-history-ai.md).

- **Where:** `src/terminal/history.rs:351`
- **Defect:** `.replace("\\n", "\n").replace("\\\\", "\\")` handles the newline
  escape before the backslash escape.
- **Failure:** `printf 'a\nb'` is stored by fish as `printf 'a\\nb'` and is
  decoded as a backslash followed by a real line break. The popup shows and
  re-inserts a broken command, and the on-disk entry no longer matches it for
  deletion.
- **Fix direction:** a single left-to-right unescape pass.

### 7. A deleted history entry reappears when it has duplicates

**Status:** fixed by [plan A](../plans/2026-11-11-review-fixes-a-graphics-history-ai.md).

- **Where:** `src/terminal/history.rs:485` (`strip_command_entries`)
- **Defect:** only the latest occurrence is removed from disk (pinned by the
  test `removal_only_deletes_single_latest_occurrence`), while the in-memory
  list is deduplicated. The rewrite changes the file mtime, so the next open
  reloads and the older duplicate surfaces.
- **Failure:** a command containing a secret that was run three times must be
  deleted three times, and is still on disk after the first delete while the UI
  showed it as gone.
- **Decision (user, 2026-11-11):** every occurrence is removed.

### 8. `Surface` has no `Drop`

**Status:** fixed by [plan B](../plans/2026-11-11-review-fixes-b-vulkan-robustness.md); Linux run pending user verification.

- **Where:** `src/render/backend/linux_gpu/surface.rs:33`
- **Defect:** framebuffers, image views, the swapchain, both semaphores, the
  fence and the `VkSurfaceKHR` are never destroyed, and `FB_EXTENTS` keeps
  their entries. `X11WindowInner::drop` then closes the `Display` under them.
- **Failure:** every closed window leaves a full set of swapchain images
  allocated for the process lifetime, which breaks the idle-RAM budget with all
  windows closed.

### 9. Partial atlas uploads transition from `UNDEFINED`

- **Where:** `src/render/backend/linux_gpu/resources.rs:420` (`write_rect`)
- **Defect:** `old_layout` is always `VK_IMAGE_LAYOUT_UNDEFINED`, which allows
  the driver to discard the whole image, although glyph uploads write a single
  cell rectangle. Each call also allocates a staging buffer and device memory
  and blocks on `vkQueueWaitIdle`. A texture that was never written is bound as
  `SHADER_READ_ONLY_OPTIMAL` while still in `UNDEFINED`.
- **Failure:** on a driver that really discards, each new glyph wipes the
  cached ones. Independently, a first screen with about 100 new glyphs performs
  about 100 synchronous GPU round-trips on the render path.
- **Fix direction:** track the current layout per texture; reuse one staging
  buffer and batch the uploads of a frame into one submission.

### 10. Non-retryable transport errors are retried ten times

**Status:** fixed by [plan A](../plans/2026-11-11-review-fixes-a-graphics-history-ai.md).

- **Where:** `src/ai/http.rs:164`
- **Defect:** every `Err` except `"Cancelled"` is retried, including
  `"Cannot start curl"`, a non-UTF-8 body, an oversized response and an invalid
  status line.
- **Failure:** with `curl` not installed, a query shows "Waiting..." for about
  4 minutes (1+2+4+8+16+32+60+60+60 s) before the error appears.
- **Fix direction:** retry only curl connection/timeout exits and HTTP
  408/429/5xx.

## Secondary findings

### Vulkan/Linux

- **X11 only.** `NativeWindow::create` (`platform/linux/native/window.rs:49`)
  always opens an X11 window. The Wayland branches in `surface.rs` and
  `platform/linux/native_clipboard.rs` are never reached, which contradicts the
  project description (Wayland and X11).
- **No IME or dead keys.** `set_ime_allowed` and `set_ime_cursor_area` are
  no-ops and key text comes from `XLookupString` without XIM
  (`platform/linux/native/x11.rs:536`), so ibus/fcitx input (for example
  Vietnamese) and compose sequences do not work.
- **Window title and class.** Only `XStoreName` is used (`x11.rs:359`,
  `x11.rs:427`): non-ASCII titles are garbled without `_NET_WM_NAME`, and
  without `WM_CLASS` desktops cannot match the window to its `.desktop` entry.
- **State shared across windows.** One command buffer (`upload_cmd`), two
  descriptor sets and the uniform buffers are global. `CommandEncoder::new`
  resets the command buffer after waiting only for the current surface's fence,
  so a frame of another window may still be executing.
- **Unchecked results.** `vkQueueSubmit` is not checked; if it fails the fence
  is never signalled and the next `acquire_frame` waits forever. Many create
  calls are unchecked, and `find_memory_type` falls back to index 0.
- **Stubs.** `current_monitor` reports 1920x1080 at 60 Hz, `theme` is always
  dark, cursor icon, cursor visibility and fullscreen only store a flag.

### `graphics/`

- **`a=d` selectors** (`kitty.rs:244`): every selector other than `i`/`I`
  clears all placements, and the uppercase forms free all images. The delete
  does not appear to be limited to the current screen, so a TUI on the
  alternate screen may wipe images in the main scrollback. Not confirmed: the
  grid-side caller was not read.
- **`t=f` existence oracle** (`kitty.rs:336`): a failed read replies
  `EPERM:access denied`, but an existing file that fails to decode replies with
  a different error, so file existence can still be probed, against the
  CWE-200 comment in the same file.
- **`I=` replies** (`kitty.rs:197`): an image sent with a number and no id gets
  an auto id, but the reply carries `i=0`.
- **APNG** (`png.rs:397`): a file whose `fcTL` frame is smaller than the canvas
  is rejected entirely instead of showing the default image.

### `history.rs`

- `HistorySearch::open` reads and parses every history file on the UI thread
  when a file changed.
- A delete is dropped silently when four deletions are already in flight
  (`history.rs:606`); the entry then returns on the next reload.

### `ai/`

- The system prompt contains `last_exit` (`ai/mod.rs:293`), so the cached
  system prefix is invalidated whenever the exit status changes.
- `PaneState::cancel_request` always records "Suggested command was skipped",
  also when no command had been suggested.

## Suggested order

1. Findings 3, 4: small, contained, each removes a crash. Finding 2 is a one-line fix of low severity.
2. Finding 1: makes inline images work on Linux.
3. Findings 5, 8, 9: backend robustness.
4. Findings 6, 7, 10 and the open question on history deletion.
5. Secondary items go to `ROADMAP.md` as follow-ups.
