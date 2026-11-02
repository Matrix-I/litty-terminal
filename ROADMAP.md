# Litty Roadmap

Release plans and historical records live in `docs/plans/<version>.md`. Completed
items, original evidence, and validation limits are preserved in those files.
New fix plans contain only issues reproduced by execution.

## Release index

| Version | Record |
| --- | --- |
| [v0.8.0](./docs/plans/0.8.0.md) | Planned follow-ups |
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
- [ ] **Needs a Linux machine: Kitty lock modifiers (Linux)**
  - `kitten show-key -m kitty`, Caps Lock on, press `a` → modifiers include `caps_lock`; off → not; Num Lock on + keypad `7` → `num_lock`
  - Native Linux container coverage: Xvfb/XTEST verifies Caps/Num Lock encoding in the running app; physical-keyboard and Wayland checks remain manual.
- [ ] **Needs a Linux machine: Quick Terminal (Linux)**
  - X11: `keybind = global:ctrl+grave_accent=toggle_quick_terminal` + reload; from another window press it → a borderless Litty panel at the top, again → hidden
  - Wayland: create a desktop shortcut running `litty +toggle-quick-terminal` → toggles the same way; with Litty not running it prints "no running Litty"
  - X11 headless coverage verifies global-hotkey show/hide, geometry, socket restoration, session reuse and autohide. Fixed initial mapping focus loss and transient global-key-grab FocusOut; autohide uses a single 50 ms deadline, then returns to normal wait scheduling. Real desktop/Wayland verification remains open.

- [ ] **Needs a Linux desktop: xdg-open smoke test**
  - Open terminal URL/path links and a release link, including spaces and hyphen-leading filenames; verify executable/script paths still open their containing directory. Command arguments and Linux compilation are covered automatically; actual desktop dispatch is not exercised here.
  - Added `linux_xdg_open_dispatch_smoke` (ignored; invoked from `scripts/linux/run.sh`): drives production `open_target` through the real `xdg-open` with a stub `$BROWSER` and asserts spaces, a hyphen-leading name (`./-dash.txt`), an executable script (opens its containing directory) and a release URL. It compiles for macOS and x86_64 Linux and **passed in the Podman container** (2026-11-01, aarch64 Debian 13, `LITTY_TEST_CA_BUNDLE=~/warp-ca.crt` fixed the earlier `static.rust-lang.org` connection error; run directly with `cargo test --locked linux_xdg_open_dispatch_smoke -- --ignored`, 1 passed). The full `scripts/linux/test.sh` still stops at Clippy: `tests/yo_shells.rs:40` triggers `clippy::unnecessary_mut_passed` on Linux (`&mut size` passed to `libc::openpty`), so the rest of `run.sh` was not reached. The handler is stubbed, so a real browser/file-manager check stays manual.

- [ ] **LLM assistant release validation on additional environments**
  - Implementation is complete (see [implementation record](./docs/plans/yosh_implementation_plan.md#implementation-record-2026-11-01)); mock provider/tool/continuation/cancellation/compaction tests and isolated zsh/Bash 3.2 smoke tests pass.
  - fish PTY interception/repaint/editable-prefill checks passed in the Linux container. Still requires Nushell runtime checks, a live-provider smoke test with configured credentials, and hardware GPU-window visual/idle-memory profiling. These are release validation, not unimplemented assistant features.
  - Anthropic live-provider smoke test **passed** (2026-11-01, `LITTY_AI_LIVE_MODEL=claude-haiku-5-5`, `bash scripts/test-ai-live.sh`, 1 passed in 9.57s): docs + scrollback tool round trips + chat; pending suggestion + edited execution + continuation; context compaction + post-compaction request; transport cancellation in 221 ms. See [scope](./docs/validation/ai-live-validation.md). Other providers (OpenAI, OpenRouter, etc.) were not live-tested.
  - Nushell 0.116.1 (aarch64 Linux container, official GitHub release binary, checksum not verified: no SHA file published at the tried URL) **passed** `python3 scripts/test-yo-shells.py` after a fix: `src/shell/litty.nu` used `char escape`, which no longer exists, so the script errored on load and never emitted `778;init;direct` (`yo` was dead on current Nushell). Now uses `char -i 27`. The harness also answers `ESC[6n` cursor queries and waits longer for Nushell's first start. Older Nushell versions (header claims 0.101+) were not tested.
  - Idle profile, macOS arm64 release build, 1 window/1 tab, no `yo` used (2026-11-01): CPU **0.0%** (CPU time flat at 0.49 s over 60 s, matches `ControlFlow::Wait`); RSS ~113 MB; `footprint` **84 MB**, of which 45 MB is IOSurface (window swapchain), ~11 MB Malloc Small, ~7 MB IOAccelerator. Meets the 85 MB `+bench-ram` budget but **not** the <50 MB invariant in `CLAUDE.md` if GPU surfaces are counted; heap alone is ~12 MB. The invariant needs a decision on what it measures. Per-session memory after `yo` is not measured.
  - Real-app keyboard checks (Ctrl+C, queued Enter, prompt repaint, editable prefill) were done by hand in a real GPU window (2026-11-02, v0.7.1). fish on real Linux is skipped for now (no Linux machine).
