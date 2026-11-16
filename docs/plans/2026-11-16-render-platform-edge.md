# Remove `render → platform` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: development-superpowers:subagent-driven-development or executing-plans-strict. Steps use `- [ ]`.

**Goal:** Drop the last `TOLERATED` layering edge, `render → platform`, so `src/render/` names no `crate::platform` path (tests included). Source: [v0.14.0](./0.14.0.md) item "Remove the tolerated `render → platform` edge".

**Architecture:** Three code commits plus a docs commit (Task 4), each of which builds on its own:

1. `render` owns a `RawHandle` enum and a `RenderWindow` trait (`src/render/window.rs`). `app` implements the trait for `platform::gui::Window` (`src/app/render_window.rs`). Both `Surface::new` backends take `&dyn RenderWindow`. A Metal guard test checks that the surface alone gives its layer two drawables.
2. Delete the redundant `configure_layer_drawables` hook. The Metal `Surface::new` already creates the view's `CAMetalLayer` with `maximumDrawableCount = 2` (`src/render/backend/metal/surface.rs:47`), so the hook sets the same value again.
3. `Renderer` holds `Weak<dyn RenderWindow>`. The Vulkan fault tests get their real window from a crate-root test helper, `src/test_window.rs`, which is listed in `TEST_INFRA`. The `TOLERATED` row is deleted.

**Tech Stack:** Rust; Metal (macOS arm64), Vulkan (Linux x86_64).

**Branch:** `refactor/render-platform-edge`

## Confirmed Requirements

1. Remove the edge entirely, not just the hook. `render` defines `RenderWindow` + `RawHandle`, `app` implements them, and the `TOLERATED` row is deleted.
2. Delete the redundant hook: the call in `Renderer::new`, `configure_layer_drawables` in `src/platform/macos/window.rs` and `src/platform/linux/window.rs`, and its export in `src/platform/mod.rs`. No behavior change is intended.
3. The Vulkan fault tests stay in `src/render/backend/linux_gpu/surface.rs`. They get their window from a crate-root test helper `src/test_window.rs`, which is added to `TEST_INFRA`.
4. Work on branch `refactor/render-platform-edge`. When done, tick the item in `docs/plans/0.14.0.md`.

## Global Constraints

From `CLAUDE.md` (project) and `~/.claude/CLAUDE.md`:

- **Layers** (enforced by `src/layering_tests.rs`): `terminal` → nothing; `render` → `terminal`; `platform` → nothing; `app` and crate-root modules → anything.
- **Hot paths:** zero heap allocation (render loop). The present path keeps `Weak::upgrade` plus a dynamic call, and neither allocates.
- **Platform parity:** the same `pub fn` exists in `macos/` and `linux/` and is exported by name from `src/platform/mod.rs`. A removal therefore happens on both platforms at once.
- **Module list:** `tests/native_activation.rs` must list every module `src/main.rs` lists (`native_activation_test_compiles_every_production_module`). Test-only modules are compared through `TEST_INFRA`.
- **Verification** before every commit touching `src/`:
  - `cargo check`
  - `cargo check --target x86_64-unknown-linux-gnu`
  - `cargo test` (outside the sandbox; PTY tests need `openpty`)
  - `cargo clippy --all-targets -- -D warnings`
  - `git diff --check`
- **Commits:** only when asked, so this plan has no `git commit` step. Use Conventional Commits, no `Co-Authored-By` trailer, and stage files explicitly.

## Technical Defaults

- **`RenderWindow: Send + Sync`.** `platform::gui::Window` is `Send + Sync` today (`unsafe impl` on both `NativeWindowInner`s), so keeping the bound leaves `Renderer`'s auto traits unchanged.
- **`inner_size` returns `(u32, u32)`.** `render` uses `PhysicalSize` nowhere else.
- **The handle mapping is a free function, not `From`.** `app::render_window::render_handle(platform::gui::RawHandle) -> render::window::RawHandle` is explicit and unit-testable.
- **`render::window::RawHandle` keeps `#[allow(dead_code)]`, like the platform enum.** Each OS reads only some variants' fields.
- **`Surface::new(window: &dyn RenderWindow, device)`.** This is the smallest backend change. In Task 1 a `&platform::gui::Window` still coerces to it.
- **The `TOLERATED` const stays as an empty slice.** `CLAUDE.md` names it as the home for exceptions.

## Risks & Rollback

| Risk | Mitigation | Revert |
|---|---|---|
| Removing the hook changes the drawable count. A third Retina drawable costs ~15 MB per window (`docs/development/performance.md:47`). | `a_configured_surface_keeps_two_drawables` (Task 1) asserts that the view's layer is the surface's own and has count 2. Task 2 also compares `+bench-ram` step 1 before and after. | Revert Task 2's commit |
| A wrong `RawHandle` mapping (a swapped field) means no Linux surface is created. | Unit test covering all 4 variants (Task 1). Vulkan fault tests on Xvfb in the container (Task 3). | Revert Task 1's commit |
| `tests/native_activation.rs` fails to build on Linux without `crate::test_window`. | Task 3 adds the `#[path]` line, and the module-sync test enforces it. | Revert Task 3's commit |
| Persisted data / config / message formats | None: only crate-internal types change. | — |

---

## Task 1: `RenderWindow` trait, `Surface::new` on it, Metal guard test

> Done 2026-11-16 — deviated: `expect(dead_code)` → `allow(dead_code)` (user chose option A; see Step 3 note). Committed as `010a7aa`.

**Files:**
- Create `src/render/window.rs`; modify `src/render/mod.rs` (add `pub(crate) mod window;` after `pub(crate) mod renderer;`)
- Create `src/app/render_window.rs`; modify `src/app/mod.rs` (add `mod render_window;` after `mod prewarm;`, alphabetical)
- Modify `src/render/backend/metal/surface.rs:3, 34` and its `mod tests`
- Modify `src/render/backend/linux_gpu/surface.rs:6, 102, 567`
- Modify `src/render/renderer/mod.rs:242` (`Surface::new(&window, …)` → `Surface::new(&*window, …)`)

**Interfaces:**
- Produces `crate::render::window::RawHandle`: `AppKit { ns_view }`, `Wayland { display, surface }`, `Xlib { display, window }`, `Xcb { connection, window }`, with the same field types as `platform::gui::RawHandle`.
- Produces `crate::render::window::RenderWindow: Send + Sync` with these methods:
  - `fn raw_handle(&self) -> Option<RawHandle>`
  - `fn scale_factor(&self) -> f64`
  - `fn inner_size(&self) -> (u32, u32)`
  - `fn pre_present_notify(&self)`
  - `fn present_skipped(&self)`
- Changes `Surface::new(window: &dyn RenderWindow, device: &Device) -> Self` on both backends.
- Produces `crate::app::render_window::render_handle(h: platform::gui::RawHandle) -> render::window::RawHandle` and `impl RenderWindow for platform::gui::Window`.
- `Renderer::new` keeps `window: Arc<platform::gui::Window>` in this task (Task 3 changes it).

- [x] **Step 0: Branch.** Run `git switch -c refactor/render-platform-edge` from an up-to-date `main`. The plan itself is already committed on `main`; Task 4 commits only its progress ticks.
- [x] **Step 1: Failing test for the handle mapping.** Create `src/app/render_window.rs` with only the test module, and declare `mod render_window;` in `src/app/mod.rs`:

```rust
//! `platform::gui::Window` as the `render` layer's window.

#[cfg(test)]
mod tests {
    use super::render_handle;
    use crate::platform::gui::RawHandle as P;
    use crate::render::window::RawHandle as R;

    fn ptr(n: usize) -> *mut std::ffi::c_void {
        n as *mut std::ffi::c_void
    }

    #[test]
    fn render_handle_keeps_every_variant_and_field() {
        assert_eq!(render_handle(P::AppKit { ns_view: ptr(1) }), R::AppKit { ns_view: ptr(1) });
        assert_eq!(
            render_handle(P::Wayland { display: ptr(2), surface: ptr(3) }),
            R::Wayland { display: ptr(2), surface: ptr(3) },
        );
        assert_eq!(render_handle(P::Xlib { display: ptr(4), window: 5 }), R::Xlib { display: ptr(4), window: 5 });
        assert_eq!(render_handle(P::Xcb { connection: ptr(6), window: 7 }), R::Xcb { connection: ptr(6), window: 7 });
    }
}
```

- [x] **Step 2: Run** `cargo test render_handle_keeps_every_variant_and_field`. Expected: FAIL to compile (`render_handle` and `crate::render::window` are not defined).
- [x] **Step 3: Create `src/render/window.rs`** and add `pub(crate) mod window;` to `src/render/mod.rs`:

```rust
//! What the renderer needs from a window. `app` implements it for the
//! platform window, so `render` does not depend on `platform`.

/// Raw native handle for GPU surface creation (Metal, Vulkan).
// Each OS reads only its own variants' fields.
#[allow(dead_code)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RawHandle {
    AppKit { ns_view: *mut std::ffi::c_void },
    Wayland { display: *mut std::ffi::c_void, surface: *mut std::ffi::c_void },
    Xlib { display: *mut std::ffi::c_void, window: std::ffi::c_ulong },
    Xcb { connection: *mut std::ffi::c_void, window: u32 },
}

pub trait RenderWindow: Send + Sync {
    /// Native handle to build the GPU surface on; `None` if the window has none.
    fn raw_handle(&self) -> Option<RawHandle>;
    // Temporary: called through the trait only once Renderer moves onto it.
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    fn scale_factor(&self) -> f64;
    /// Physical size in pixels `(width, height)`.
    // Temporary: called through the trait only once Renderer moves onto it.
    #[allow(dead_code)]
    fn inner_size(&self) -> (u32, u32);
    /// Right before a frame is presented (Wayland asks for its frame callback here).
    // Temporary: called through the trait only once Renderer moves onto it.
    #[allow(dead_code)]
    fn pre_present_notify(&self);
    /// The frame announced by `pre_present_notify` was not presented.
    // Temporary: called through the trait only once Renderer moves onto it.
    #[allow(dead_code)]
    fn present_skipped(&self);
}
```

  Only the backend `Surface::new`s call the trait in this task: Metal calls `raw_handle` and `scale_factor`, Linux calls only `raw_handle`. `Renderer` still calls the inherent `Window` methods. In this binary crate every trait is crate-private, so a trait method nothing calls is reported as `dead_code`. The `allow`s are temporary, and Task 3 Step 3 removes them. `expect` was tried first and rejected (deviation, 2026-11-16): `tests/native_activation.rs` reported the three unconditional ones as `unfulfilled_lint_expectations`, which fails `clippy -D warnings`.

- [x] **Step 4: Implement `src/app/render_window.rs`** above the test module:

```rust
use crate::platform::gui::{RawHandle as PlatformHandle, Window};
use crate::render::window::{RawHandle, RenderWindow};

/// The same handle, as the `render` layer's type.
pub(crate) fn render_handle(handle: PlatformHandle) -> RawHandle {
    match handle {
        PlatformHandle::AppKit { ns_view } => RawHandle::AppKit { ns_view },
        PlatformHandle::Wayland { display, surface } => RawHandle::Wayland { display, surface },
        PlatformHandle::Xlib { display, window } => RawHandle::Xlib { display, window },
        PlatformHandle::Xcb { connection, window } => RawHandle::Xcb { connection, window },
    }
}

impl RenderWindow for Window {
    fn raw_handle(&self) -> Option<RawHandle> {
        Window::raw_handle(self).map(render_handle)
    }

    fn scale_factor(&self) -> f64 {
        Window::scale_factor(self)
    }

    fn inner_size(&self) -> (u32, u32) {
        let size = Window::inner_size(self);
        (size.width, size.height)
    }

    fn pre_present_notify(&self) {
        Window::pre_present_notify(self);
    }

    fn present_skipped(&self) {
        Window::present_skipped(self);
    }
}
```

  (`Window::x(self)` resolves to the inherent method, so this does not recurse.)

- [x] **Step 5: Switch the backends.**
  - `src/render/backend/metal/surface.rs`: line 3 → `use crate::render::window::{RawHandle, RenderWindow};`; line 34 → `pub fn new(window: &dyn RenderWindow, device: &Device) -> Self {`. Lines 35 and 51 do not change.
  - `src/render/backend/linux_gpu/surface.rs`: line 6 → `use crate::render::window::{RawHandle, RenderWindow};`; line 102 → `pub fn new(window: &dyn RenderWindow, device: &Device) -> Self {`. Keep the `_ => {}` arm (`AppKit` is still a variant).
  - The Linux `mod faults` still builds a `platform::gui::Window` (`use super::*` no longer brings `Window` in). Add `Window` to its existing line 567 import: `use crate::platform::gui::{LogicalSize, Window, WindowSpec};`. Task 3 removes that import. At line 604, `Surface::new(&window, &device)` coerces `&Window` to `&dyn RenderWindow` unchanged.
  - `src/render/renderer/mod.rs:242`: `Surface::new(&window, &device)` → `Surface::new(&*window, &device)` (`&Window` → `&dyn RenderWindow`).
- [x] **Step 6: Run** `cargo test render_handle_keeps_every_variant_and_field`. Expected: PASS.
- [x] **Step 7: Metal guard test.** In `src/render/backend/metal/surface.rs` `mod tests`, after `dropping_the_surface_detaches_its_layer_from_the_view`, add:

```rust
    /// A bare test view as the renderer's window.
    struct ViewWindow(Id);
    // SAFETY: the test uses the view on its own thread only.
    unsafe impl Send for ViewWindow {}
    unsafe impl Sync for ViewWindow {}

    impl RenderWindow for ViewWindow {
        fn raw_handle(&self) -> Option<RawHandle> {
            Some(RawHandle::AppKit { ns_view: self.0 })
        }
        fn scale_factor(&self) -> f64 {
            2.0
        }
        fn inner_size(&self) -> (u32, u32) {
            (20, 20)
        }
        fn pre_present_notify(&self) {}
        fn present_skipped(&self) {}
    }

    /// The view shows only the surface's own layer, double-buffered, with
    /// no help from the platform layer: two Retina drawables are ~15 MB each.
    #[test]
    fn a_configured_surface_keeps_two_drawables() {
        let (device, _queue) = crate::render::backend::shared_device_and_queue();
        unsafe {
            let rect = NSRect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 };
            let view = msg_send!(class(c"NSView").unwrap(), c"alloc"; -> Id);
            let view = msg_send!(view, c"initWithFrame:", rect => NSRect; -> Id);
            let window = ViewWindow(view);
            let mut surface = Surface::new(&window, &device);
            let config = SurfaceConfig { width: 20, height: 20, format: surface.default_format(), transparent: false };
            surface.configure(&device, &config);

            let layer = msg_send!(view, c"layer"; -> Id);
            assert_eq!(layer, surface.layer.as_ptr(), "the view shows the surface's own layer");
            assert_eq!(msg_send!(layer, c"maximumDrawableCount"; -> usize), 2);

            drop(surface);
            msg_send!(view, c"release");
        }
    }
```

- [x] **Step 8: Run** `cargo test a_configured_surface_keeps_two_drawables`. Expected: PASS. This is a characterization guard. To prove it can fail, temporarily change `2usize` to `3usize` at `surface.rs:47` and run again. Expected: FAIL `left: 3, right: 2`. Then revert that line.
- [x] **Step 9: Run** `cargo check && cargo check --target x86_64-unknown-linux-gnu && cargo clippy --all-targets -- -D warnings && cargo clippy --all-targets --target x86_64-unknown-linux-gnu -- -D warnings && cargo test && git diff --check`. Expected: PASS. The Linux clippy checks the Linux-only `cfg_attr(expect)`. If that target cannot be linted on this host, run it in the container. The `TOLERATED` row is still present, so `render_layer_uses_only_terminal` passes.
- [x] **Commit checkpoint:** Ready to commit `src/render/window.rs src/render/mod.rs src/app/render_window.rs src/app/mod.rs src/render/backend/metal/surface.rs src/render/backend/linux_gpu/surface.rs src/render/renderer/mod.rs` with message `"refactor(render): build gpu surfaces from a render-owned window trait"`. Body: `render defines RenderWindow and RawHandle; app implements them for platform::gui::Window. Adds a guard that a configured Metal surface keeps two drawables.`

## Task 2: Delete the redundant `configure_layer_drawables` hook

> Done 2026-11-16. `+bench-ram` step 1 footprint: 59.00 MB → 58.00 MB. Export was on `src/platform/mod.rs:30` (plan said 31; mechanical). Committed as `fc05973`.

**Files:**
- Modify `src/render/renderer/mod.rs:254`: delete `crate::platform::configure_layer_drawables(&window);`
- Modify `src/platform/macos/window.rs:12-36`: delete the doc comment and the whole function. On line 1 of the module doc, `..., title-bar zoom, Metal drawables.` → `..., title-bar zoom.`
- Modify `src/platform/linux/window.rs:3`: delete `pub fn configure_layer_drawables(...) {}` and the blank line after it.
- Modify `src/platform/mod.rs:31`: remove `configure_layer_drawables, ` from the `pub use` list.

**Interfaces:**
- Consumes the guard `a_configured_surface_keeps_two_drawables` from Task 1.
- Produces: `platform::configure_layer_drawables` no longer exists. Its only caller was `Renderer::new`; confirm with `grep -rn configure_layer_drawables src`.

- [x] **Step 1: Baseline RAM (macOS, before this task's edits).** Run `cargo build --release && ./target/release/litty +bench-ram` and record the step 1 "footprint" (1 window, 1 tab). For reference, `docs/development/performance.md:39` says 63 MB.
- [x] **Step 2: Apply the four deletions listed above.**
- [x] **Step 3: Run** `cargo check && cargo check --target x86_64-unknown-linux-gnu && cargo clippy --all-targets -- -D warnings && git diff --check`. Expected: PASS.
  - `src/platform/macos/window.rs` keeps `use crate::platform::gui::Window;` for `apply_window_config`.
  - Delete only imports that clippy reports as unused.
- [x] **Step 4: Run** `cargo test`. Expected: PASS, including `a_configured_surface_keeps_two_drawables`.
- [x] **Step 5: Run** `cargo build --release && ./target/release/litty +bench-ram`. Expected: step 1 footprint within ±2 MB of Step 1. A rise of 10 MB or more means a third drawable: stop and report.
- [x] **Commit checkpoint:** Ready to commit `src/render/renderer/mod.rs src/platform/macos/window.rs src/platform/linux/window.rs src/platform/mod.rs` with message `"refactor(render): drop redundant metal drawable hook"`. Body: `Surface::new already sets maximumDrawableCount = 2 on the layer it attaches; the hook set it again. Guarded by a_configured_surface_keeps_two_drawables. +bench-ram step 1: <before> MB -> <after> MB.`

## Task 3: `Renderer` on `RenderWindow`; remove the `TOLERATED` row

> Done 2026-11-16. Linux container: 820 passed, 0 failed, 17 ignored. Committed as `370f9bd`.

**Files:**
- Modify `src/layering_tests.rs:12-18` (`TEST_INFRA`, `TOLERATED`)
- Modify `src/render/renderer/mod.rs:9, 134, 233-242`
- Modify `src/render/window.rs` (remove Task 1's temporary `allow(dead_code)` attributes)
- `src/render/renderer/frame.rs:591-599`: no change; check that it compiles.
- Modify `src/render/backend/linux_gpu/surface.rs:567-604` (`mod faults`)
- Create `src/test_window.rs`; modify `src/main.rs` (add `#[cfg(test)]` + `mod test_window;` next to `mod test_alloc;`)
- Modify `tests/native_activation.rs` (add `#[path = "../src/test_window.rs"] mod test_window;` after the `test_alloc` line)
- Modify `src/render/mod.rs` module doc

**Interfaces:**
- Consumes `RenderWindow` and `RawHandle` from Task 1.
- Changes `Renderer::new(window: Arc<dyn RenderWindow>, font: Arc<FontSet>, font_px: f32, bg_opacity: f32, cursor_style: u32) -> Self`. The only caller, `src/app/window.rs:125`, passes `window.clone()` (`Arc<Window>`); it coerces at the argument site without a source change.
- Produces (test, Linux only) `crate::test_window::native_window(title: &'static str, width: f64, height: f64) -> Option<Box<dyn RenderWindow>>`.

- [x] **Step 1: Failing test: delete the tolerated row.** In `src/layering_tests.rs`:

```rust
/// Crate-root modules any layer may use from `#[cfg(test)]` code.
const TEST_INFRA: &[&str] = &["test_alloc", "test_window"];

/// `(layer, used crate module, why it is still allowed)`.
const TOLERATED: &[(&str, &str, &str)] = &[];
```

- [x] **Step 2: Run** `cargo test render_layer_uses_only_terminal`. Expected: FAIL "render may only depend on terminal". It lists `src/render/renderer/mod.rs` and `src/render/backend/linux_gpu/surface.rs` with `crate::platform`. The test reads files and ignores `cfg`, so the Linux file is scanned on macOS too.
- [x] **Step 3: Switch `Renderer`** in `src/render/renderer/mod.rs`:
  - Line 9: `use crate::platform::gui::Window;` → `use crate::render::window::RenderWindow;`
  - Line 134: `window: std::sync::Weak<Window>,` → `window: std::sync::Weak<dyn RenderWindow>,`
  - Lines 233–242:

```rust
    pub fn new(
        window: Arc<dyn RenderWindow>,
        font: Arc<FontSet>,
        font_px: f32,
        bg_opacity: f32,
        cursor_style: u32,
    ) -> Self {
        let (window_width, window_height) = window.inner_size();
        let (device, queue) = gpu::shared_device();
        let mut surface = Surface::new(&*window, &device);
```

  - The `extent` line: `let extent = extent::Extent::new(window_width, window_height);`
  - `Arc::downgrade(&window)` and `window.scale_factor()` stay as they are. `frame.rs:591-599` compiles unchanged, because `upgrade()` yields `Arc<dyn RenderWindow>` with the same method names.
  - In `src/render/window.rs`, delete the four temporary attributes and their `// Temporary:` comments: the `cfg_attr(…, allow(dead_code))` on `scale_factor` and the three `#[allow(dead_code)]` on `inner_size`, `pre_present_notify` and `present_skipped`. All five methods are now called through the trait on both OSes. Check with `grep -n 'dead_code\|Temporary' src/render/window.rs`: the only hit should be the enum's `#[allow(dead_code)]`.

- [x] **Step 4: Test window helper.** Create `src/test_window.rs`:

```rust
//! Real windows for tests in layers that may not name `platform`.
#![cfg(target_os = "linux")]

use crate::platform::gui::{LogicalSize, Window, WindowSpec};
use crate::platform::linux::native::window::NativeWindow;
use crate::render::window::RenderWindow;

/// A visible, decorated native window (X11 or Wayland, as `NativeWindow`
/// picks) of `width` x `height` logical pixels; `None` when none can be made.
pub fn native_window(title: &'static str, width: f64, height: f64) -> Option<Box<dyn RenderWindow>> {
    let spec = WindowSpec {
        title,
        decorations: true,
        visible: true,
        transparent: false,
        inner_size: LogicalSize::new(width, height),
        position: None,
    };
    let window = Window::from_native_linux(NativeWindow::create(&spec).ok()?);
    Some(Box::new(window))
}
```

  In `src/main.rs`, next to `mod test_alloc;`:

```rust
#[cfg(test)]
mod test_window;
```

  The attribute must be exactly `#[cfg(test)]` on its own line, or `production_modules` counts the module as production code. In `tests/native_activation.rs`, add `#[path = "../src/test_window.rs"] mod test_window;` after the `test_alloc` line.

- [x] **Step 5: Switch the fault rig** in `src/render/backend/linux_gpu/surface.rs` `mod faults`:
  - Delete both `use crate::platform::…` lines (567–568).
  - Change the `Rig` field `_window: Window,` to `_window: Box<dyn RenderWindow>,`.
  - Replace the `let spec = WindowSpec { … };` block and the `let window = Window::from_native_linux(…)` line with:

```rust
            let window = crate::test_window::native_window("litty-vulkan-fault-test", 200.0, 100.0)?;
            let mut surface = Surface::new(&*window, &device);
```

  The `DISPLAY` check and the skip messages stay as they are.

- [x] **Step 6: Module doc.** In `src/render/mod.rs`, change `plus the tolerated edges listed there.` → `plus test infrastructure listed there.`
- [x] **Step 7: Run** each of these:
  - `cargo test render_layer_uses_only_terminal`
  - `cargo test native_activation_test_compiles_every_production_module`
  - `grep -rn "crate::platform" src/render`

  Expected: both tests PASS, and grep prints nothing.
- [x] **Step 8: Run** `cargo check && cargo check --target x86_64-unknown-linux-gnu && cargo clippy --all-targets -- -D warnings && cargo clippy --all-targets --target x86_64-unknown-linux-gnu -- -D warnings && cargo test && git diff --check`. Expected: PASS. The Linux clippy checks the Linux-only `cfg_attr(expect)`. If that target cannot be linted on this host, run it in the container.
- [x] **Step 9: Linux tests in the container.** Use the zigbuild + amd64 podman/lavapipe/Xvfb recipe (memory "Linux container GUI testing"; CA bundle `/Users/linh.nguyen/warp-ca.crt`). Run `cargo test` there with `DISPLAY` set. Done: 820 passed, 0 failed, 17 ignored (debian:trixie-slim, Xvfb + lavapipe; all 12 `faults` tests ran).
  - Expected: the `faults` tests run without the "no X display" skip, and `a_healthy_frame_is_submitted_and_presented` and the rest PASS.
  - If the container cannot run, report "Vulkan fault tests not run on Linux".
- [x] **Commit checkpoint:** Ready to commit `src/layering_tests.rs src/render/window.rs src/render/renderer/mod.rs src/render/backend/linux_gpu/surface.rs src/render/mod.rs src/test_window.rs src/main.rs tests/native_activation.rs` with message `"refactor(render): drop the tolerated render to platform edge"`. Body: `Renderer keeps a Weak<dyn RenderWindow>; Vulkan fault tests get their window from the test_window helper. TOLERATED is now empty.`

## Task 4: Verification & Docs

- [x] **Step 1: Docs.**
  - `docs/plans/0.14.0.md:42` (was at line 46 at execution time; mechanical): change `- [ ]` → `- [x]` and append `Done 2026-MM-DD on refactor/render-platform-edge (plan: [2026-11-16-render-platform-edge](./2026-11-16-render-platform-edge.md)); the Metal hook was redundant and is deleted.`
  - `docs/plans/0.8.0.md:49`: change `- [ ]` → `- [x]` and append ` Done in v0.14.0 ([plan](./2026-11-16-render-platform-edge.md)).`
  - `src/layering_tests.rs` module doc: no change; `TOLERATED` still exists.
  - `README.md` / `ROADMAP.md`: no change. Nothing user-visible changes, and `ROADMAP.md` only links v0.14.0.
- [x] **Step 2: Full gate** (after the last edit). Quote each command and its result.
  - `cargo check`
  - `cargo check --target x86_64-unknown-linux-gnu`
  - `cargo test` (outside the sandbox)
  - `cargo clippy --all-targets -- -D warnings`
  - `git diff --check`
- [x] **Step 3: End-to-end on macOS.** Done 2026-11-16: the user ran the smoke test by hand and reported it ok. Run `cargo build --release && ./target/release/litty`, then open a window, type `echo ok`, resize, open a second tab, and close.
  - Evidence: the text renders, a resize shows no blank frame, and `+bench-ram` step 1 is within ±2 MB of Task 2's Step 1 baseline.
  - GUI automation has no permissions here, so the user must run this smoke test by hand. Report it as unverified until they confirm.
- [x] **Step 4: End-to-end on Linux.** The container `cargo test` from Task 3 Step 9 proves only build and tests, not a GUI run. Report a real Wayland/X11 session as unverified. Unverified: no real Wayland/X11 session was run.
- [x] **Step 5: Read the full diff** (`git diff main...`, untracked files included). Check for anything unrelated, debug output, TODOs, or code made dead. `platform::gui::RawHandle` stays in use through `app::render_window`.
- [x] **Step 6:** Run `code-review-strict` (non-trivial, multi-file change) and address or report its findings. If anything changes, repeat from Step 2. Done: one round, Approved; two Minor doc findings fixed (stale commit notes, 0.8.0.md wording). Docs-only, so no re-review.
- [x] **Commit checkpoint:** Ready to commit `docs/plans/0.14.0.md docs/plans/0.8.0.md docs/plans/2026-11-16-render-platform-edge.md` with message `"docs(plans): close render to platform layering item"`.
