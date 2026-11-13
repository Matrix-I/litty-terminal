# Litty Roadmap

Release plans and historical records live in `docs/plans/<version>.md`. Completed
items, original evidence, and validation limits are preserved in those files.
Fix items record execution evidence and its limits. Maintenance proposals and
optimization candidates are identified separately from reproduced failures.

## Release index

| Version | Record |
| --- | --- |
| [v0.13.0](./docs/plans/0.13.0.md) | Planned — every open item carried over from earlier records and this roadmap, to resolve in one release |
| [v0.12.0](./docs/plans/0.12.0.md) | Release record (2026-11-13) — Linux Vulkan robustness and X11 integration (IME, monitors, theme, fullscreen, titles), security and robustness fixes, 10 MB scrollback default, alt-screen and non-ASCII parser speedups |
| [v0.11.1](./docs/plans/0.11.1.md) | Release record (2026-11-11) — source review fixes (click-to-move panic, RIS, OSC titles, scroll margins), OSC 8 file-link hardening, faster OSC 8 interning and rows-only resize, SIGWINCH after resize bursts, macOS live-resize and window fixes |
| [v0.11.0](./docs/plans/0.11.0.md) | Release record (2026-11-09) — ships the performance gap plan: new latency, startup and PTY throughput benchmarks, shaping cache, faster OSC 8 / Unicode paths, overlapped startup and lower idle memory; T1 and emoji interning carry over |
| [v0.10.1](./docs/plans/0.10.1.md) | Release record (2026-11-08) — macOS pointer-hide, key-window and Kitty function-key fixes; first release built by the tag-triggered CI workflow |
| [v0.10.0](./docs/plans/0.10.0.md) | Release record (2026-11-08) — ships the direct Metal/Vulkan renderer, native AppKit/X11 windowing, native OS font/image engines and the SIMD VT/search core; native Wayland, the macOS `winit` benchmark and the size/RAM targets carry over (listed in the record) |
| [v0.9.0](./docs/plans/0.9.0.md) | Release record (2026-11-05) — ships the v0.8.1 fixes, tab drag-and-drop and the Linux title bar |
| [v0.8.1](./docs/plans/0.8.1.md) | Historical record — shipped in v0.9.0 (never tagged) |
| [v0.8.0](./docs/plans/0.8.0.md) | Release record (2026-11-04) — ships the v0.7.3 and v0.7.4 items; open follow-ups are listed in the record |
| [v0.7.4](./docs/plans/0.7.4.md) | Historical record — items 2.1–2.8 shipped in v0.8.0 (never tagged); Linux feature validation confirmed; historical measurement limits retained |
| [v0.7.3](./docs/plans/0.7.3.md) | Historical record — 6/6 shipped in v0.8.0 (never tagged); Linux feature validation confirmed; macOS UI verified by the user (2026-11-12) |
| [v0.7.2](./docs/plans/0.7.2.md) | Release record — runtime-confirmed bug, security and platform fixes |
| [v0.7.1](./docs/plans/0.7.1.md) | Historical release record |
| [v0.7.0](./docs/plans/0.7.0.md) | Historical release record |
| [v0.6.0](./docs/plans/0.6.0.md) | Historical release record |
| [v0.5.0](./docs/plans/0.5.0.md) | Historical release record |
| [v0.4.1](./docs/plans/0.4.1.md) | Historical release record |
| [v0.4.0](./docs/plans/0.4.0.md) | Historical release record |
| [v0.3.0](./docs/plans/0.3.0.md) | Tag record; no roadmap at this release |
| [v0.2.0](./docs/plans/0.2.0.md) | Tag record; no roadmap at this release |
| [v0.1.0](./docs/plans/0.1.0.md) | Tag record; no roadmap at this release |

The unversioned feature and manual-test catalog is filed under v0.4.0, where a
roadmap first exists in the local release tags. The unversioned memory audit is
filed under v0.4.1, where that section first appears. These groupings do not
assert exact introduction versions for every individual item.

Detailed structural refactor plans and the Yosh implementation plan remain in
`docs/plans/` and are linked from their release records. Outstanding structural
follow-ups are in v0.8.0; shipped refactor results are in v0.7.1.

## Verification debt

- [x] Manually test Kitty keys in the real app: letters, `Ctrl`/`Alt`+letter, `Esc`, `Tab` vs `Ctrl+I`, key repeat (verify `key_without_modifiers()` on macOS)
- [x] Manually test OSC 8 hover (`Cmd`/`Ctrl` + mouse move underlines the link, preview pill shows the URI)
- [x] **Linux feature and stability validation** (user-confirmed, 2026-11-04)
  - All features have been tested on Linux and run stably. Linux runtime/desktop verification is closed and is no longer technical debt.
  - This supersedes the outstanding Kitty lock modifier, Quick Terminal (X11/Wayland), real `xdg-open`, and fish-on-Linux checks. See the [validation status](./docs/validation/linux-status-2026-11-04.md) for evidence scope.

- [x] **Intermittent 47 MB footprint with all windows closed (macOS)**
  - Closed as not reproduced (2026-11-12): 20 more runs at `a307298` under load 10–26 gave 31 MB (16), 33 MB (2) and 40 MB (2, at the two highest loads), never 47 MB; all under the 50 MB budget. Reopen if a 0-window run exceeds 50 MB. See the [close-out report](./docs/validation/2026-11-12-macos-closeout.md).
  - During v0.11.1 validation (2026-11-11) the 0-window footprint stayed at 47 MB in 7 of 18 runs of the release build, the level from before the drawable fix `79d44ef`; v0.11.0 gave 28–32 MB in 8 alternating runs. 78 later runs (v0.11.0, `15bca71`, `d37a085`, `a0f7ebc`, `86967b8` and the release build) never showed it, so it is not reproduced and no fix was made. Suspected timing under system load. Method and data: [validation report](./docs/validation/release-0.11.1.md).

- [x] **Occlusion events are never emitted (macOS and Linux)**
  - Scheduled for [v0.13.0](./docs/plans/0.13.0.md).
  - Found in the 2026-11-11 performance review: no platform sends `WindowEvent::Occluded`, so `State::occluded` is always `false`. Covered or minimized windows keep rendering on PTY output and their drawables are never shrunk. Needs an emitter in `src/platform/{macos,linux}/` and its own plan. See [finding 2](./docs/validation/2026-11-11-perf-review.md).
  - macOS fixed (2026-11-12): the window delegate handles `windowDidChangeOcclusionState:` and reports `Occluded` when `occlusionState` loses `NSWindowOcclusionStateVisible` (covered, minimized, hidden, other Space). A hidden window keeps its grid size while its drawables are shrunk (`render/renderer/extent.rs`) and is not drawn on PTY output. Unit-tested; the running app (minimize, cover, switch Space, native tabs, Quick Terminal, a minimized `vim` across a config reload) was checked by the user on 2026-11-12.
  - Linux fixed (2026-11-13): X11 map/unmap, `_NET_WM_STATE_HIDDEN` and `VisibilityNotify` (`FullyObscured`) report `Occluded`; a minimized window under GNOME (XWayland) went from 34–35 to 3 CPU ticks per 5 s with a shell printing ~100 lines/s. Covering by another window was not checked (the compositor reports `Unobscured`). See [v0.13.0](./docs/plans/0.13.0.md).

- [x] **Plans B and C of the 2026-11-11 review (Vulkan) need a run on real Linux hardware**
  - Closed (2026-11-13): the maintainer confirms every `fix(linux)` commit shipped in v0.12.0 was verified on real Linux hardware.
  - The code is done and passes in an x86_64 container (lavapipe, Xvfb, validation layers); the manual checklists in [plan B](./docs/plans/2026-11-11-review-fixes-b-vulkan-robustness.md) and [plan C](./docs/plans/2026-11-11-review-fixes-c-vulkan-images.md) are open. The macOS GUI runs of the perf plan's Step 3 items 1–4 and plan A's Task 5 Step 5 were checked by the user on 2026-11-12 and pass.

- [ ] **LLM assistant release validation on additional environments**
  - Not scheduled: descoped from [v0.13.0](./docs/plans/0.13.0.md#deferred) on 2026-11-13 as nice to have; planned again only if an issue is opened.
  - Implementation is complete (see [implementation record](./docs/plans/yosh_implementation_plan.md#implementation-record-2026-11-01)); mock provider/tool/continuation/cancellation/compaction tests and isolated zsh/Bash 3.2 smoke tests pass.
  - Linux feature validation is complete per the user confirmation above. Separate provider/version coverage and quantitative memory measurements remain in the v0.8.0 follow-ups. These are release validation, not unimplemented assistant features.
  - Anthropic live-provider smoke test **passed** (2026-11-01, `LITTY_AI_LIVE_MODEL=claude-haiku-5-5`, `bash scripts/test-ai-live.sh`, 1 passed in 9.57s): docs + scrollback tool round trips + chat; pending suggestion + edited execution + continuation; context compaction + post-compaction request; transport cancellation in 221 ms. See [scope](./docs/validation/ai-live-validation.md). Other providers (OpenAI, OpenRouter, etc.) were not live-tested.
  - Nushell 0.116.1 (aarch64 Linux container, official GitHub release binary, checksum not verified: no SHA file published at the tried URL) **passed** `python3 scripts/test-yo-shells.py` after a fix: `src/shell/litty.nu` used `char escape`, which no longer exists, so the script errored on load and never emitted `778;init;direct` (`yo` was dead on current Nushell). Now uses `char -i 27`. The harness also answers `ESC[6n` cursor queries and waits longer for Nushell's first start. Older Nushell versions (header claims 0.101+) were not tested.
  - Idle profile, macOS arm64 release build, 1 window/1 tab, no `yo` used (2026-11-01): CPU **0.0%** (CPU time flat at 0.49 s over 60 s, matches `ControlFlow::Wait`); RSS ~113 MB; `footprint` **84 MB**, of which 45 MB is IOSurface (window swapchain), ~11 MB Malloc Small, ~7 MB IOAccelerator. Meets the 85 MB `+bench-ram` budget but **not** the <50 MB invariant in `CLAUDE.md` if GPU surfaces are counted; heap alone is ~12 MB. The invariant needs a decision on what it measures. Per-session memory after `yo` is not measured.
  - Real-app keyboard checks (Ctrl+C, queued Enter, prompt repaint, editable prefill) were done by hand in a real GPU window (2026-11-02, v0.7.1). Linux feature checks are now complete per user confirmation (2026-11-04).

## Review follow-ups (2026-11-11)

From [the 2026-11-11 review](./docs/reviews/2026-11-11-linux-history-ai-graphics.md). Not scheduled.

- [ ] **Linux: Wayland window backend.** `NativeWindow::create` is X11 only; the Wayland surface and clipboard paths are unreachable.
  - Moved to its own plan and branch (decided 2026-11-13): [Wayland backend](./docs/plans/wayland-backend.md). Not part of v0.13.0.
  - Phase 1 (window and surface) done on `feat/wayland-window` (2026-11-13), opt-in with `LITTY_BACKEND=wayland`: an `xdg_toplevel` (app id `litty`) on one shared connection, Vulkan surface, configure, resize, title, close, and redraws paced by frame callbacks. Checked on Ubuntu GNOME (mutter): the window opens and draws, idle costs 1 CPU tick per 5 s (cursor blink), and a headless Weston smoke test (`scripts/linux/wayland_smoke.py`) checks open, a compositor-driven resize, frame callbacks and close. Terminal output is paced too: `yes` in the window presents ~60 frames/s, each after its frame callback. Known cost: each presented frame allocates the `wl_callback` proxy wayland-rs creates for the frame request (no way to reuse one). Input, scale, decorations, text input and occlusion are the later phases; X11 stays the default.
  - Phase 2 (input) done on `feat/wayland-input` (2026-11-14): keyboard through the compositor's xkb keymap (libxkbcommon loaded at run time with `xkbcommon-dl`, so the binary still links only libc), keys named by keysym so remaps such as `caps:escape` apply, client key repeat on a timerfd (and the compositor's own, `wl_keyboard` v10), pointer motion, buttons and wheel/touchpad scroll, and cursors through `wp_cursor_shape_v1` with a cursor-theme fallback. Checked on Ubuntu GNOME (mutter): the keymap compiles and the cursor shape is set on enter; a headless sway smoke test (`scripts/linux/wayland_sway_smoke.py`, `wtype`/`wlrctl`) checks shifted text, Ctrl, client key repeat and cursor-shape binding. Not yet: dead keys and Compose (phase 5), horizontal scroll.
  - Phase 3 (scale and outputs) done on `feat/wayland-scale` (2026-11-14): `wp_fractional_scale_v1` through a `wp_viewport` (integer scale as the fallback, scale 1 without a viewporter), pointer and touchpad positions in buffer pixels, and `current_monitor` from the output the window is on (current mode size and refresh rate). Checked on Ubuntu GNOME at 125%: a 968x620 window draws a 1210x775 buffer and reports 60 Hz; the sway smoke test checks a 150% output (682x426 logical, 1023x639 buffer). Not yet checked: moving between monitors with different scales.
- [x] **Linux: IME and dead keys.** `set_ime_allowed` is a no-op and X11 text comes from `XLookupString` without XIM.
  - Fixed on X11 (2026-11-12): windows connect to the XIM server `XMODIFIERS` names (on-the-spot preedit, candidate window at the cursor) and fall back to Xlib's built-in method for dead keys and Compose; a server that starts late or restarts is picked up. Checked in the real app under XWayland (GNOME) with synthetic keys: IBus Unikey Telex `tieengs vieetj` commits `tiếng việt` with preedit `a`→`â` (`main`: raw letters), `dead_acute e` gives `é` and `Multi_key o c` gives `©` (`main`: `e`, `oc`); late start, crash and restart of `ibus-x11` all recover, in every open window (2026-11-12, two windows: `tiếng`/`việt` after a late start, `aé` while it is down, `đâ` after a restart). Xlib's input method calls are serialised process-wide (parallel tests crashed in libX11), the candidate-window spot is sent from the event loop rather than while rendering, and keycode-0 presses another client sends are not looked up (libX11's built-in method reads an uninitialised compose-tree entry for them). Since then dead keys and Compose typed with `XSendEvent` no longer compose (the `é`/`©` check above used them; hardware keys and XTEST are unaffected but could not be injected here under GNOME on Wayland). Not supported (descoped 2026-11-13): Xlib returns the composed text of a sent key as a sent keycode-0 press, which cannot be told from a forged one, and looking those up would bring back the libX11 crash; real keyboards and XTEST-based tools are expected to compose (not checked, see above). The settings portal is reached only at `DBUS_SESSION_BUS_ADDRESS` or `$XDG_RUNTIME_DIR/bus`, never through D-Bus autolaunch. GNOME on Wayland does not run `ibus-x11`, so there IBus reaches Litty only once it is started by hand or through the Wayland backend (text-input), which is part of the item above.
- [x] **Linux: `_NET_WM_NAME` and `WM_CLASS`.** Non-ASCII titles are garbled and desktops cannot match the window to its `.desktop` entry.
  - Fixed (2026-11-12): `WM_CLASS` is `litty`, `litty` (`91de8c7`); titles are also set as `_NET_WM_NAME` (`UTF8_STRING`) on creation and on every title change. Checked with `xprop` on the real app under XWayland (GNOME): an OSC 2 title `litty e2e — Tiếng Việt ✓ 日本語` is read back unchanged; on `main` the property is missing. `WM_NAME` is now encoded per ICCCM (2026-11-13): `STRING` when Latin-1 holds the title, `COMPOUND_TEXT` otherwise, so window managers that ignore EWMH show it too (read back under Xvfb; a real non-EWMH window manager is still to be checked on Linux).
- [x] **Linux: per-window Vulkan frame state.** One command buffer, two descriptor sets and the uniform buffers are shared by all windows. The validation layers report the command buffer being reset, begun and submitted while still pending (`VUID-vkResetCommandBuffer-commandBuffer-00045`, `-00049`, `-00071`) when a second window opens (2026-11-12, lavapipe).
  - Fixed (2026-11-12) by waiting instead of duplicating the state: a frame or upload first waits for the last frame submitted by any window (`VulkanCore::wait_for_last_frame`). With one window that fence was already waited for when the image was acquired, so nothing changes; with several, a window's recording waits for the other window's GPU work. Reproduced on an Intel UHD 620 (Mesa, XWayland, Khronos validation layer 1.4.341): opening a second window on `main` gave 10 each of `-00045`, `-00049`, `-00071` and 2 of `VUID-VkPresentInfoKHR-pImageIndices-01430`; on the fix, none in three runs. Per-window state would let windows record while another's frame runs; not needed for a terminal's frame times.
- [x] **Linux: one `render_done` semaphore per surface.** It is signaled again while the presentation engine may still wait on it (`VUID-vkQueueSubmit-pSignalSemaphores-00067`, with one window too; 2026-11-12, Intel UHD 620). Needs one semaphore per swapchain image.
  - Fixed (2026-11-12, `bb2a680`): one semaphore per swapchain image, picked by the acquired image index. Two windows under the validation layer on `main` gave 8 of `-00067`; on the fix, none.
- [x] **Linux: check `vkQueueSubmit` and creation results**; pick the physical device by present support and device type.
  - Fixed (2026-11-12, `5ba9871`, `702148d`): every creation, submit and present result is checked; a failed setup step falls back or skips the draw with a `litty: ... failed (VkResult n)` line instead of using a null handle. The device is the first non-CPU one with a graphics queue, swapchain support and SPIR-V 1.1 (or the storage-buffer extension), software renderers last. The device is picked before any window exists, so present support is checked when a window's surface is created. Fault-injection tests cover setup, buffers, textures, swapchains and every frame step (2026-11-13, see [v0.13.0](./docs/plans/0.13.0.md)).
- [x] **Linux: custom shaders** (GLSL to SPIR-V, offscreen targets).
  - Done (2026-11-13): the Shadertoy snippet is wrapped as GLSL 450 (uniforms as push constants, `iChannel0` the previous pass's render target) and compiled to SPIR-V with `naga` when the config loads; compile errors name the line in the user's file. Render targets end their pass ready to be sampled, and every pass orders its drawing against the sampling around it. Checked in the real app (XWayland, GNOME): an invert shader turned the window from mean RGB (1, 1, 1) to (254, 254, 254), right side up. See [v0.13.0](./docs/plans/0.13.0.md).
- [x] **Linux: batch glyph uploads** into one submission per frame. A new sampled texture also costs two submissions (layout transition, then the first upload).
  - Fixed (2026-11-12, `fe5fd57`): uploads and layout transitions record into a batch submitted with the frame. `glyph_upload_cost` (release, Intel UHD 620): 64 new glyphs 8.1 ms to 1.2 ms, 1024 new glyphs 131 ms to 8 ms.
- [x] **Linux: `shader.spv` needs Vulkan 1.1.** It declares `SPV_KHR_storage_buffer_storage_class` while the instance asks for Vulkan 1.0 without that extension (`VUID-VkShaderModuleCreateInfo-pCode-08742`).
  - Fixed (2026-11-12, `5ba9871`): the instance asks for Vulkan 1.1 when the loader has it, and a 1.0 device enables `VK_KHR_storage_buffer_storage_class`. `main` gave 2 of `-08742` with two windows; the fix, none.
- [x] **Linux: `precise_scroll_gpu_pixels` crashes.** The ignored GPU test passes a texture view as a framebuffer and segfaults with a real Vulkan device.
  - Fixed (2026-11-12, `aa1b7b9`): render-target textures own a framebuffer with offscreen render passes, and an unknown target records no pass. The test segfaulted on `main` and now passes with its pixel checks and no validation errors.
- [x] **Linux: `custom-shader` warning.** It is printed once per window and worded as a compile error; say once that the key is unsupported.
  - Fixed (2026-11-12, `ab2de85`): one `custom-shader is not supported on Linux yet` line per process, however many windows open. Gone since custom shaders run on Linux (2026-11-13).
- [x] **Graphics: unsupported Kitty `a=d` selectors** return an error without changing image data or placements (2026-11-12). Alternate-screen scoping done 2026-11-13: deletions touch only the active screen's placements, and `A`/`I` free only unused images transmitted on that screen; `d=I` with `p=` removes only that placement; same-id placements on the two screens no longer replace each other (see [v0.13.0](./docs/plans/0.13.0.md#graphics)).
- [x] **Graphics: `t=f`/`t=t` failure replies** use the same `EPERM` for read, decompression and image-decoding failures (2026-11-12).
- [x] **Graphics: `I=` replies and APNG fallback**: numbered transmissions reply with the assigned id; valid APNGs with later partial frames show the full default image while malformed default images remain rejected (2026-11-12).
- [x] **History: background reload and bounded deletes**: history reloads offload to a background worker; deletions are queued and processed sequentially without silent drops or corrupting rewrites (2026-11-12).
- [x] **Linux: window stubs.** `current_monitor` reports 1920x1080 at 60 Hz, `theme` is always dark, `set_fullscreen` only stores a flag.
  - Partly fixed (2026-11-12): the X11 window now shows the requested cursor icon (`23ba626`) and hides the pointer while typing (`e30ab0f`); maximize and fullscreen changes made by the window manager are read from `_NET_WM_STATE` (`7652831`).
  - Fixed (2026-11-12): `current_monitor` reads RandR (the monitor under the window centre; a 3072x1728 panel reports 59941 mHz where `main` reported 60000). `theme` and `system_dark` follow the XDG settings portal `color-scheme` (no preference, like a missing portal, keeps the dark default), and light/dark switches reach the app live: switching GNOME to Light and back sends `CSI ?997;2n` then `;1n` to a pane in mode 2031 (`main`: nothing). `set_fullscreen` asks the WM through `_NET_WM_STATE` (checked against mutter by an ignored test); no app action calls it yet.
- [x] **AI: stable system prefix and cancellation**: exit status travels in changing user context, and cancellation records a skipped command only once when a suggestion existed (2026-11-12).
