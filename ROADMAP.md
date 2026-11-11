# Litty Roadmap

Release plans and historical records live in `docs/plans/<version>.md`. Completed
items, original evidence, and validation limits are preserved in those files.
Fix items record execution evidence and its limits. Maintenance proposals and
optimization candidates are identified separately from reproduced failures.

## Release index

| Version | Record |
| --- | --- |
| [v0.11.1](./docs/plans/0.11.1.md) | Release record (2026-11-11) — source review fixes (click-to-move panic, RIS, OSC titles, scroll margins), OSC 8 file-link hardening, faster OSC 8 interning and rows-only resize, SIGWINCH after resize bursts, macOS live-resize and window fixes |
| [v0.11.0](./docs/plans/0.11.0.md) | Release record (2026-11-09) — ships the performance gap plan: new latency, startup and PTY throughput benchmarks, shaping cache, faster OSC 8 / Unicode paths, overlapped startup and lower idle memory; T1 and emoji interning carry over |
| [v0.10.1](./docs/plans/0.10.1.md) | Release record (2026-11-08) — macOS pointer-hide, key-window and Kitty function-key fixes; first release built by the tag-triggered CI workflow |
| [v0.10.0](./docs/plans/0.10.0.md) | Release record (2026-11-08) — ships the direct Metal/Vulkan renderer, native AppKit/X11 windowing, native OS font/image engines and the SIMD VT/search core; native Wayland, the macOS `winit` benchmark and the size/RAM targets carry over (listed in the record) |
| [v0.9.0](./docs/plans/0.9.0.md) | Release record (2026-11-05) — ships the v0.8.1 fixes, tab drag-and-drop and the Linux title bar |
| [v0.8.1](./docs/plans/0.8.1.md) | Historical record — shipped in v0.9.0 (never tagged) |
| [v0.8.0](./docs/plans/0.8.0.md) | Release record (2026-11-04) — ships the v0.7.3 and v0.7.4 items; open follow-ups are listed in the record |
| [v0.7.4](./docs/plans/0.7.4.md) | Historical record — items 2.1–2.8 shipped in v0.8.0 (never tagged); Linux feature validation confirmed; historical measurement limits retained |
| [v0.7.3](./docs/plans/0.7.3.md) | Historical record — 6/6 shipped in v0.8.0 (never tagged); Linux feature validation confirmed; macOS UI verification still pending |
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

- [ ] **Intermittent 47 MB footprint with all windows closed (macOS)**
  - During v0.11.1 validation (2026-11-11) the 0-window footprint stayed at 47 MB in 7 of 18 runs of the release build, the level from before the drawable fix `79d44ef`; v0.11.0 gave 28–32 MB in 8 alternating runs. 78 later runs (v0.11.0, `15bca71`, `d37a085`, `a0f7ebc`, `86967b8` and the release build) never showed it, so it is not reproduced and no fix was made. Suspected timing under system load. Method and data: [validation report](./docs/validation/release-0.11.1.md).

- [ ] **Occlusion events are never emitted (macOS and Linux)**
  - Found in the 2026-11-11 performance review: no platform sends `WindowEvent::Occluded`, so `State::occluded` is always `false`. Covered or minimized windows keep rendering on PTY output and their drawables are never shrunk. Needs an emitter in `src/platform/{macos,linux}/` and its own plan. See [finding 2](./docs/validation/2026-11-11-perf-review.md).

- [ ] **LLM assistant release validation on additional environments**
  - Implementation is complete (see [implementation record](./docs/plans/yosh_implementation_plan.md#implementation-record-2026-11-01)); mock provider/tool/continuation/cancellation/compaction tests and isolated zsh/Bash 3.2 smoke tests pass.
  - Linux feature validation is complete per the user confirmation above. Separate provider/version coverage and quantitative memory measurements remain in the v0.8.0 follow-ups. These are release validation, not unimplemented assistant features.
  - Anthropic live-provider smoke test **passed** (2026-11-01, `LITTY_AI_LIVE_MODEL=claude-haiku-5-5`, `bash scripts/test-ai-live.sh`, 1 passed in 9.57s): docs + scrollback tool round trips + chat; pending suggestion + edited execution + continuation; context compaction + post-compaction request; transport cancellation in 221 ms. See [scope](./docs/validation/ai-live-validation.md). Other providers (OpenAI, OpenRouter, etc.) were not live-tested.
  - Nushell 0.116.1 (aarch64 Linux container, official GitHub release binary, checksum not verified: no SHA file published at the tried URL) **passed** `python3 scripts/test-yo-shells.py` after a fix: `src/shell/litty.nu` used `char escape`, which no longer exists, so the script errored on load and never emitted `778;init;direct` (`yo` was dead on current Nushell). Now uses `char -i 27`. The harness also answers `ESC[6n` cursor queries and waits longer for Nushell's first start. Older Nushell versions (header claims 0.101+) were not tested.
  - Idle profile, macOS arm64 release build, 1 window/1 tab, no `yo` used (2026-11-01): CPU **0.0%** (CPU time flat at 0.49 s over 60 s, matches `ControlFlow::Wait`); RSS ~113 MB; `footprint` **84 MB**, of which 45 MB is IOSurface (window swapchain), ~11 MB Malloc Small, ~7 MB IOAccelerator. Meets the 85 MB `+bench-ram` budget but **not** the <50 MB invariant in `CLAUDE.md` if GPU surfaces are counted; heap alone is ~12 MB. The invariant needs a decision on what it measures. Per-session memory after `yo` is not measured.
  - Real-app keyboard checks (Ctrl+C, queued Enter, prompt repaint, editable prefill) were done by hand in a real GPU window (2026-11-02, v0.7.1). Linux feature checks are now complete per user confirmation (2026-11-04).

## Review follow-ups (2026-11-11)

From [the 2026-11-11 review](./docs/reviews/2026-11-11-linux-history-ai-graphics.md). Not scheduled.

- [ ] **Linux: Wayland window backend.** `NativeWindow::create` is X11 only; the Wayland surface and clipboard paths are unreachable.
- [ ] **Linux: IME and dead keys.** `set_ime_allowed` is a no-op and X11 text comes from `XLookupString` without XIM.
- [ ] **Linux: `_NET_WM_NAME` and `WM_CLASS`.** Non-ASCII titles are garbled and desktops cannot match the window to its `.desktop` entry.
- [ ] **Linux: per-window Vulkan frame state.** One command buffer, two descriptor sets and the uniform buffers are shared by all windows.
- [ ] **Linux: check `vkQueueSubmit` and creation results**; pick the physical device by present support and device type.
- [ ] **Linux: custom shaders** (GLSL to SPIR-V, offscreen targets).
- [ ] **Linux: batch glyph uploads** into one submission per frame.
- [ ] **Graphics: Kitty `a=d` selectors** other than `i`/`I` clear everything; confirm the alternate-screen behaviour against the grid caller.
- [ ] **Graphics: `t=f` replies** still reveal whether a file exists (decode errors differ from `EPERM`).
- [ ] **Graphics: `I=` replies** carry `i=0` instead of the assigned id; APNG files with partial frames are rejected.
- [ ] **History: reload on the UI thread** when a history file changed; deletes dropped silently when four are in flight.
- [ ] **Linux: window stubs.** `current_monitor` reports 1920x1080 at 60 Hz, `theme` is always dark, cursor icon, cursor visibility and fullscreen only store a flag.
- [ ] **AI: `last_exit` in the system prompt** invalidates the cached prefix; `cancel_request` records a skipped command when none was suggested.
