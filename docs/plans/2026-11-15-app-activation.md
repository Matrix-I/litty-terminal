# macOS App Activation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: development-superpowers:subagent-driven-development or executing-plans-strict. Steps use `- [ ]`.

**Goal:** Click a desktop notification to return to its originating live pane;
explicitly reuse a running macOS process to open a new window at the caller's cwd.
**Architecture:** A bounded same-user local activation channel carries typed
new-window and exact-pane requests to the application event loop. Native
UserNotifications callbacks forward to the exact process session. Application
state alone selects panes and creates windows. Ordinary launches remain separate.
**Tech Stack:** Rust/std/libc, existing Objective-C runtime integration,
AppKit and UserNotifications. No new Rust dependencies.
**Branch:** `feat/app-activation`, based on `af3b5ed`.

## Confirmed Requirements

Confirmed by the maintainer on 2026-11-15:

1. macOS only. Linux keeps its notification and startup behavior; new platform
   APIs have matching Linux twins and explicit unsupported/no-op behavior.
2. A click selects the exact originating window/tab/pane, revealing hidden tabs
   and splits. A closed pane is ignored; no replacement shell is created.
3. Reuse is opt-in and creates a new window inside an existing process, with
   the caller's cwd. Normal invocations create independent processes.
4. No eligible instance: start a new instance. Several: choose the most recently
   started eligible instance. Delivery timeout: report an error, never silently
   fall back to opening another window. Benchmarks/utilities stay local.
5. Complete the plan, commit it, then implement without another execution-choice
   handoff (maintainer: "xong plan thì implement luôn đi nhé"). Planning commits
   are authorized. No push, release or merge of the implementation is authorized.
6. Wayland preview is complete. Linux notification actions and Wayland token
   forwarding are outside this phase; macOS uses AppKit activation.

## Global Constraints

The following project requirements apply verbatim:

- **Performance**: < 50MB idle RAM (macOS `footprint`, app idle with all windows closed; open windows add GPU swapchain surfaces and are budgeted by `+bench-ram`), 0% idle CPU (`ControlFlow::Wait`), zero heap allocation on hot paths (PTY parsing, scrolling, render loop).
- **Grid Cell**: `Cell` struct must stay exactly 12 bytes (`static_assert`). Pack new state into existing bits.
- **Concurrency & Locking**: Minimize grid `Mutex` hold time. Never perform blocking I/O (PTY writes, subprocesses, file ops) while holding it.
- Only async-signal-safe calls after `fork()`; prepare all allocations beforehand.
- Strip `ESC` (`\x1b[201~`) from pasted text to prevent bracketed paste escapes.
- User/terminal URLs to `open` / `xdg-open` must pass as a single argument (never via shell).
- Shell history writes: mutate raw bytes, write temp file, atomic `rename`.
- Layers: `terminal` → nothing; `render` → `terminal`; `platform` → nothing;
  `app` and crate-root modules → anything. Do not extend layering exceptions.
- OS integration goes in `src/platform/{macos,linux}/` with the same `pub fn`
  in both and is exported by name from `src/platform/mod.rs`.
- Commit only when asked. Stage explicit files; no force push, tag, release or
  unrelated changes. Implementation commit checkpoints below are proposals only.
- No new keybindings, dependencies, persisted settings, shell commands, shell
  environments supplied by IPC, or changes to the 12-byte grid cell.

Required verification after the last source edit:

```sh
cargo check
cargo check --target x86_64-unknown-linux-gnu
cargo test
cargo clippy --all-targets -- -D warnings
git diff --check
```

`cargo test` runs outside the sandbox for PTY access. Read the full diff,
untracked files included; self-review requirements, concurrency, error paths,
performance, security, naming/API compatibility, tests and design; request
`code-review-strict`; address findings and repeat the final gate after edits.

## Technical Defaults

- CLI spellings: `+reuse` and `--reuse`, matching `src/cli.rs`'s existing paired
  spellings. No arguments other than the invoking cwd are forwarded. This is a
  proposed additive CLI format, explicitly covered by this implementation plan.
- Normal GUI instances advertise eligibility; benchmark runs never advertise.
  Client dispatch occurs before prewarm, AppKit initialization or PTY creation.
- Use an owner-only directory under the macOS temporary filesystem and one
  private socket per process session. Reject symlinks, incorrect UID/mode,
  non-socket entries and oversized frames. Validate the peer UID on both ends.
- A random session identity prevents PID/pane-id reuse across restarts. Order
  instances by an OS monotonic start stamp, not filesystem modification times,
  wall-clock time, or discovery order. No polling while idle.
- Versioned request envelopes carry a session, request ID and either a cwd or
  pane ID. There is no arbitrary command/env execution operation. Bounds:
  16 KiB frame, 4 KiB cwd, 4096 registry entries per client scan, one active transport worker
  per server, 2 s transport deadline and 64 cached request outcomes retained through each request's monotonic expiry. Expired frames are rejected; live outcomes are never evicted to admit another request. Reject
  overflow visibly. Requests are deduplicated; acknowledge window creation
  only after the application has successfully created it.
- Notification payloads contain process-session routing and pane identity, not
  terminal content as executable instructions. Responses arriving in another
  same-bundle process forward to the exact origin, never the newest instance.
- Use UserNotifications, not deprecated NSUserNotification. Authorization is
  requested only when a terminal notification first needs delivery; denial is
  reported without repeated prompts. Preserve Linux's helper path.
- Native implementation may use a small Objective-C ARC bridge compiled by a
  macOS-only `build.rs` with the installed Apple SDK, without adding crates.
  Linux builds must not invoke Apple tools. Test both Apple architectures.
- AppKit activation is cooperative. Use current selectors when available and
  retain a guarded macOS 11 compatibility path. Never promise focus policy
  can be overridden. Passive app activation must not select the first window.

## Risks & Rollback

| Risk | Mitigation | Rollback |
| --- | --- | --- |
| Cold notification relaunch creates an unwanted shell | Characterize real bundle launch before changing startup; install the UN delegate before `finishLaunching`; distinguish explicit open intent from a notification-only launch | Keep the task incomplete if the real no-shell guarantee cannot be demonstrated; do not weaken it |
| Wrong process receives a notification response | Route exact session and pane via IPC; reject stale session; test two instances and restarts | Restore existing notification helper |
| Wrong pane after tab tear-off/native-tab selection | Resolve stable pane ID against current app state at click time; reveal/unminimize selected window | Revert selection handler |
| Spoofed, stalled or duplicate IPC | Same-user checked endpoints, bounded reads/time/queues, deduplication, exact-origin routing, error on uncertain delivery | Remove additive reuse dispatch; ordinary launches continue |
| Cwd disappears before shell launch | Validate at boundary and ensure the requested directory is used; report failure rather than silently opening at home | Revert cwd-specific startup path |
| Permission/bundle identity unavailable | Explicit error, no fake successful delivery; test packaged app and bare executable | Existing helper remains fallback only where its lack of actions is documented |
| Native bridge changes packaging/toolchain | macOS-only build step; deployment target 11; arm64/x86_64 checks; Linux cross-check | Revert bridge/build step together |
| Idle CPU/RAM regression | Event-driven listener and deterministic resource cleanup; measure closed-window footprint | Disable activation service and revert |

There is no persisted data migration. IPC is ephemeral and versioned; reject
incompatible peers explicitly. Do not delete unrelated stale files or silently
change existing command/config formats. Runtime errors must not log terminal
notification bodies, raw frame contents, or private paths.

## Code Survey

Anchors below refer to `3481fa3`; use symbol anchors if preceding tasks move lines.

| Files / anchors | Current behavior / planned integration |
| --- | --- |
| `src/cli.rs:1–105`, `Command`, `parse`, tests below 107 | Paired `+`/`--` dispatch; add explicit reuse variant and tests |
| `src/main.rs:43–141`, `main` | Dispatch reuse before ordinary `app::run` |
| `src/app/mod.rs:47–94`, `App`, `open_window`; 292–350 `resumed`; 531–567 `run`, `run_app` | App owns windows; startup currently creates initial PTY before callback registration |
| `src/app/user_event.rs:98–126`, `on_reopen` | Selects first window; must not run for passive activation/notification selection |
| `src/app/terminal_io.rs:40–70`, `handle_terminal_requests` | Drains per-pane notification outside grid lock but passes title/body only |
| `src/platform/process.rs:43–66`, `show_notification` | Existing osascript/notify-send helper |
| `src/platform/macos/app.rs:25–62`, `register_reopen_handler`; 192–201 `activate_app` | Both Dock reopen and did-become-active emit Reopen; deprecated activation selector |
| `src/platform/macos/native/app.rs:354–384`, `run_native` | `finishLaunching` precedes app construction; move early native notification setup before it |
| `src/platform/macos/objc.rs`, `msg_send`, `autoreleasepool` | Current direct Objective-C ABI helpers |
| `src/platform/macos/native/window.rs:496–505,533–538` | Existing minimize/focus helpers |
| `src/platform/types.rs:69–80`, `PlatformEvent`, `EventSink`; `src/event.rs:12–64`, `AppEvent`, conversion | Keep platform independent of app; typed events cross boundary |
| `src/platform/mod.rs`, both OS `mod.rs` / `app.rs` | Explicit API exports and platform counterparts |
| `src/tab.rs:12–16,63–104`, `next_id`, `Tab` | Pane IDs are process-local and survive moves; zoom can hide target |
| `src/app/window.rs:41–45`, `create_window` | Accepts cwd; app must check requested cwd errors |
| `src/terminal/pty.rs:230–234,383` | Existing fallback/unchecked chdir must not silently violate reuse cwd |
| `resources/Info.plist`, `bundle_macos.sh` | Bundle ID `com.litty.terminal`, minimum macOS 11; bundle for real native tests |
| `src/layering_tests.rs`, `TOLERATED` | No new layer exception allowed |

New files are explicitly identified below; they have no existing line ranges.

## Official API References

- [UN delegate registration before launch completion](https://developer.apple.com/documentation/usernotifications/unusernotificationcenter/delegate).
- [Notification authorization](https://developer.apple.com/documentation/usernotifications/unusernotificationcenter/requestauthorization(options:completionhandler:)).
- [AppKit cooperative activation](https://developer.apple.com/documentation/appkit/passing-control-from-one-app-to-another-with-cooperative-activation).
- [Launch user-info keys](https://developer.apple.com/documentation/appkit/nsapplicationdidfinishlaunching-user-info-keys).

## Shared Interfaces and Ownership

The common types below belong in new `src/platform/activation.rs`:
`ActivationAction`, `ActivationRequest` (including `expires_ns: u64`),
`ActivationReply`, `ActivationSink`, and the bounded codec/cache appendix.
The OS twins export `ActivationServer`, `start_activation` and `reuse_window`.
The macOS server provides `origin(&self) -> &str` and
`session(&self) -> [u8; 16]`; origin is a validated endpoint basename, never an
arbitrary path. Exact-origin forwarding is
`forward_notification(origin: &str, pane: u64) -> std::io::Result<()>` on a
bounded background worker. It must never use newest-instance discovery.

Notification twins expose
`init_notifications() -> std::io::Result<()>` before finishLaunching and
`show_pane_notification(origin: &str, pane: u64, title: &str, body: &str)
-> std::io::Result<()>`. Linux initialization is a documented no-op and delivery
retains the existing helper behavior. New APIs are explicitly exported by name.
A macOS-only `startup_is_notification() -> bool` must be backed by the Task 1
experiment; it is not implemented by assuming an NSUserNotification launch key
is valid for UNNotificationResponse. App startup reads the result before PTY
creation. Native notification delegates/blocks cannot hold app references.

## Task 1 — Characterize native launch and define bounded activation contracts

Files: new `src/platform/activation.rs`, new
`src/platform/macos/activation.rs`, Linux twin, macOS-only notification bridge
and build support if required; existing `src/platform/mod.rs` and OS exports.

Interfaces produced (private crate APIs; no app types inside platform):

```rust
pub enum ActivationAction { NewWindow(String), Pane(u64) }
pub struct ActivationRequest {
    pub id: [u8; 16],
    pub session: [u8; 16],
    pub expires_ns: u64,
    pub action: ActivationAction,
}
pub type ActivationReply = Result<(), String>;
pub type ActivationSink = std::sync::Arc<dyn Fn(
    ActivationRequest, std::sync::mpsc::SyncSender<ActivationReply>
) + Send + Sync>;
```

- [ ] Add a native bundle characterization in `scripts/macos/activation_probe.m` with runner `scripts/macos/test-activation-probe.sh` that logs launch-kind and callback
  ordering without notification title/body. Observe normal launch, first
  notification authorization/delivery, live click, click delivered to another
  process and click after the origin exits. Capture whether a modern UN cold
  launch is distinguishable before default PTY creation. Do not assume the
  old `NSApplicationLaunchUserNotificationKey` carries a UN response.
- [ ] Add pure wire tests before implementation: exact round trip for Unicode
  cwd and pane IDs; truncated/header/version/oversize/malformed requests
  rejected; no embedded NUL; no relative cwd; duplicate IDs do not repeat an
  action; session mismatch never dispatches. Native completion blocks fire
  once on success, ignored action, malformed payload and stale target.
- [ ] Run targeted tests red, retaining the failure output. The smallest core
  cwd boundary test is:

```rust
#[test]
fn reuse_cwd_rejects_relative_nul_and_oversized_input() {
    assert!(validate_cwd("relative").is_err());
    assert!(validate_cwd("/tmp/\0bad").is_err());
    assert!(validate_cwd(&format!("/{}", "x".repeat(4096))).is_err());
    assert_eq!(validate_cwd("/tmp/tiếng việt").unwrap(), "/tmp/tiếng việt");
}
```

Minimal boundary implementation:

```rust
pub fn validate_cwd(cwd: &str) -> std::io::Result<&str> {
    if cwd.len() > 4096 || cwd.as_bytes().contains(&0)
        || !std::path::Path::new(cwd).is_absolute()
    {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput,
            "invalid activation directory"));
    }
    Ok(cwd)
}
```

- [ ] Implement the typed envelope and codec with explicit bounded sizes; add
  corresponding checked native string parsing. Re-run the tests green.
- [ ] Preserve the launch characterization as an integration test/validation
  record. If modern cold-launch classification cannot be proven, stop this
  task with the exact observation; do not implement a timer-based guess.

Ready to commit: task files — `feat(macos): define activation request boundaries`.

## Task 2 — Same-user transport, discovery and reuse client

Files: new macOS activation transport module, existing platform exports and
Linux activation twin. Tests colocated with protocol/transport. Consume Task 1
contracts. Produce:

```rust
pub fn start_activation(sink: ActivationSink) -> std::io::Result<ActivationServer>;
pub fn reuse_window(cwd: &str) -> std::io::Result<bool>;
// true: acknowledged by existing instance; false: no eligible instance.
// Error: delivery uncertain or explicitly rejected; caller must not fall back.
```

`ActivationServer` owns endpoint/session, shutdown signal and worker join handle;
Drop wakes and joins the worker and removes only its own socket/directory.

- [ ] Write failing transport tests with real private temporary sockets:
  newest among two eligible instances wins regardless of enumeration order;
  missing endpoint returns false; stale/incompatible endpoint is classified;
  wrong owner/mode/symlink rejected; oversized/truncated frame rejected; a
  repeated ID gets the cached result; invalid cwd rejected; timeout after
  delivery returns error and no fallback action; dropping server cleans its
  endpoint and wakes its worker. Coordinate tests with channels/barriers,
  never sleeps or shared environment mutation.
- [ ] Implement bounded discovery and monotonic metadata. Advertise only
  initialized ordinary GUI instances. Validate peer identity, exact session and
  frame before enqueueing; cap candidate count and response cache. No polling
  thread, background directory scan, unbounded queue, or worker per connection.
- [ ] A request consumes one admission slot until the app replies. Cache the
  result by request ID. ACK only after creating the requested window. If a
  deadline expires, report uncertainty and do not retry or fall back.
- [ ] Linux twin returns an explicit unsupported error for reuse; ordinary
  Linux startup and notification helpers remain unchanged. Run transport and
  boundary suites green and the Linux cross-check.

Ready to commit: task files — `feat(macos): add bounded instance activation transport`.

## Task 3 — CLI dispatch and real app window/pane activation

Files: `src/cli.rs`, `src/main.rs`, `src/event.rs`, `src/app/mod.rs`,
`src/app/user_event.rs`, new `src/app/activation.rs`; use existing tab/window
helpers. Consume transport/sink; produce an app event holding the request and
reply sender, `run_reuse() -> std::io::Result<()>` and pane-selection handler.

- [ ] Add CLI regression tests before the new variant:

```rust
#[test]
fn reuse_spellings_are_explicit_and_benchmarks_stay_local() {
    assert_eq!(parse(&argv(&["+reuse"])), Command::Reuse);
    assert_eq!(parse(&argv(&["--reuse"])), Command::Reuse);
    assert_eq!(parse(&argv(&[])), Command::Run {
        bench_ram: false, bench_all: false, scroll_fps: false,
    });
    assert_eq!(parse(&argv(&["+bench-features"])), Command::BenchFeatures);
}
```

Minimal parser change: add `Reuse` to `Command`; after `is` is constructed,
return `Command::Reuse` when `is(&["+reuse", "--reuse"])`.

- [ ] Run `cargo test cli::tests` red, then apply parser/dispatch and rerun green.
  Dispatch gets cwd with an explicit UTF-8/absolute-path error. A true client
  result exits success; false starts the normal GUI with that cwd; error exits
  nonzero. Do this before `prewarm::start` or any default PTY creation.
- [ ] Add real app-state tests: pane in inactive tab selected; conflicting zoom
  cleared; moved pane selected at its current location; closed pane and stale
  session leave window count unchanged; native tab/window unminimized and
  focused; passive did-become-active does not call generic `on_reopen`.
- [ ] Implement the event-loop handler. Resolve pane by scanning current windows
  and tabs. Set active tab and pane, clear conflicting zoom, call `sync_size`,
  restore target window, invalidate renderer and request redraw. Preserve
  explicit Dock reopen behavior. Do not mark another window focused by hand.
- [ ] NewWindow validates cwd exists/is a directory and creates via
  `create_window(event_loop, &waker, Some(cwd))`; send success only after the
  window and PTY exist. Ensure cwd failure cannot silently become home via the
  current PTY fallback; test a directory deleted between request and creation.
  Scope any PTY change to preserving a requested cwd and propagate its error.
  Open the requested directory before fork (owned descriptor, close-on-exec),
  use async-signal-safe `fchdir` in the child, and report child setup/exec errno
  over a close-on-exec error pipe to the parent. Parent success requires EOF
  after exec, not just successful fork. Bound the handshake and reap on error.
  Tests cover missing path, regular file, deleted/renamed directory descriptor
  semantics and child setup failure without acknowledging a successful window.
- [ ] Ordinary runs advertise a server; utility/benchmark runs do not. Keep
  zero-window ordinary instances eligible. Release server resources on exit.
  Run CLI, app selection, transport and affected PTY suites green.

Ready to commit: task files — `feat(macos): route reuse and pane activation through app state`.

## Task 4 — Native notifications, exact-origin routing and lifecycle

Files: new `src/platform/macos/notifications.rs` and Linux twin; native
Objective-C bridge/build support if Task 1 requires it; `src/platform/types.rs`,
OS exports, `src/platform/macos/native/app.rs`, `src/platform/macos/app.rs`,
`src/app/terminal_io.rs`, `src/platform/process.rs`.

Consume initialized session/transport and app pane handler. Produce matching
platform APIs for early notification initialization and pane-targeted delivery.

- [ ] Add failing callback tests for default action, dismiss, malformed/oversized
  identity, session from an exited process, closed pane, click arriving in the
  other process, and repeated callback. Native completion handler is called
  exactly once on every branch; Rust callbacks never unwind across C ABI.
- [ ] Initialize/retain the UN delegate before `finishLaunching`, as required by
  Apple. Do not replace the existing AppKit delegate or lose Dock/appearance
  callbacks. Keep delivery lazy and bound pending authorization work.
- [ ] On first notification request authorization for alerts, then submit
  `UNNotificationRequest` with a unique identifier containing process session
  and notification sequence. Payload routing includes exact origin and pane.
  Bound pending notifications and report delivery/permission failures without
  logging terminal text. Bare executables lacking bundle identity must fail
  explicitly or use the documented existing non-actionable helper path.
- [ ] Callback parses only the application's bounded routing payload, completes
  the native callback promptly and forwards through a bounded routing worker.
  Exact local session dispatches the pane action; another session routes to its
  specific endpoint, never discovery's newest instance. Missing origin returns
  without opening a window. Dismiss has no activation effect.
- [ ] Apply Task 1's experimentally verified notification-only startup handling.
  No default shell, generic Reopen or passive activate callback may create a
  window for a stale notification. On shutdown withdraw session notifications
  and release delegates/blocks/workers deterministically. A restarted process
  must reject old session identity even if its first pane ID is again 1.
- [ ] Use macOS 14+ `activate` where available and older guarded compatibility
  selector for macOS 11–13. Verify minimized, hidden and native-tab windows.
- [ ] Keep notification extraction/rate limiting outside the grid lock; pass pane
  identity to the platform. Preserve Linux's helper arguments and behavior.
  Run boundary, callback, app-state and full affected module suites green.

Ready to commit: task files — `feat(macos): activate originating panes from notifications`.

## Task 5 — Verification & Docs

Files: `README.md`, `ROADMAP.md`, this plan, new
`docs/validation/macos-app-activation-2026-11-15.md`, integration tests and
scripts added by the preceding tasks.

- [ ] Run every project verification command above, plus release build and
  `cargo check --target x86_64-apple-darwin`. Report all failures and ignored
  tests. No real desktop assertion may be replaced by a mocked call count.
- [ ] Exercise the packaged app: notification from an inactive pane, moved tab,
  minimized/hidden/native-tab window, click in another same-bundle process,
  closed pane, origin exit and process restart. Record resulting pane identity
  and prove window/shell count does not increase for stale notification clicks.
- [ ] Exercise CLI with zero, one and two instances; Unicode/spaced cwd; verify
  new shell cwd through its PTY; stop the selected instance during delivery;
  freeze it after delivery to trigger timeout and show no duplicate fallback.
  Ordinary CLI launches and benchmarks retain their original process behavior.
- [ ] Measure idle CPU and macOS footprint with all windows closed; check no
  periodic transport wakeups, no leaked workers, no stale live endpoints.
- [ ] Document paired reuse CLI spellings, selection policy, error/no-fallback
  semantics, permission/bundle limitations, platform scope and focus-policy
  limits. Link this feature separately from completed Wayland preview.
- [ ] Read full diff and untracked files; remove unrelated/debug/secret content.
  Self-review each project gate with evidence; no silent business assumptions.
- [ ] Invoke `code-review-strict` on the complete change after verification;
  fix/triage findings, repeat verification/review after edits. Do not claim an
  unavailable strict reviewer ran. Report actual desktop/architecture gaps.

Ready to commit: task files — `docs(macos): document app activation and validation`.

## Plan Review and Workflow Limits

- Self-review: requirements map to Tasks 1–5; macOS minimum, platform parity,
  exact-origin routing, idempotency, timeouts, stale targets, cwd, idle cost,
  startup order, docs and verification are explicit.
- Supplemental design review completed in this session; it identified early
  delegate registration, passive-activation focus theft, process-session IDs,
  moved panes, cwd fallback and cold-launch characterization. Incorporated above.
- The strict plan-review template mandates `model: opus`; that model is not
  exposed by the collaboration tool. No external Claude/source upload is
  authorized or attempted. Supplemental review cannot be called Opus approval.
- Reviewer findings about replay horizon, cross-task signatures, concrete
  characterization harness and parent-visible cwd failure have been addressed
  above. Supplemental reviewer still needs the final implementation package.
- Task 1 probe ran from an ad-hoc signed bundle on this Mac. Normal launch
  reports `NSApplicationLaunchIsDefaultLaunchKey = 1`; notification authorization
  returns `UNErrorDomain Code=1` (not allowed for this application). Thus live
  click and cold-start ordering remain blocked by notification authorization;
  no notification-only startup strategy is approved or presumed correct.
  The probe has no PTY and cannot execute a shell. Do not change system
  notification settings until the maintainer authorizes that test.
- Full literal native/transport implementation listings are not present in this
  draft. This does not satisfy the skill's full-code-per-task requirement;
  executable implementations and red/green evidence must be added during work.
  Do not claim this planning requirement passed. The unresolved native launch
  characterization is an explicit first-task gate, not an assumed behavior.
- The maintainer explicitly requested implementation after planning, so continue
  without an execution-method question or routine batch approval. Stop only for
  an actual unresolved behavior, unsafe change or technical blocker.

## Execution Order and Current Gates

The native cold-launch experiment blocks notification startup integration only.
Bounded contracts, transport and opt-in reuse are independent and may proceed
while permission is pending. Do not mark Task 1's native acceptance or Task 4
complete until real notification delivery is available. The native implementation
cannot be fully prescribed before the launch-order experiment; its missing full
listing remains a strict planning limitation, not an approval to guess.

## Appendix — Complete Bounded Contract Implementation

New `src/platform/activation.rs`; extract tests and prove red before copying
production code. Export visibility/dead-code annotations must match platform use.

```rust
//! Bounded activation messages. No terminal or application dependencies.
use std::io::{self, Read, Write};

pub const MAX_CWD: usize = 4096;
pub const MAX_FRAME: usize = 16 * 1024;
pub const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActivationAction {
    NewWindow(String),
    Pane(u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivationRequest {
    pub id: [u8; 16],
    pub session: [u8; 16],
    pub expires_ns: u64,
    pub action: ActivationAction,
}

pub type ActivationReply = Result<(), String>;
pub type ActivationSink = std::sync::Arc<
    dyn Fn(ActivationRequest, std::sync::mpsc::SyncSender<ActivationReply>) + Send + Sync,
>;

pub fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid activation message")
}

pub fn validate_cwd(cwd: &str) -> io::Result<&str> {
    if cwd.len() > MAX_CWD
        || cwd.as_bytes().contains(&0)
        || !std::path::Path::new(cwd).is_absolute()
    {
        return Err(invalid());
    }
    Ok(cwd)
}

impl ActivationRequest {
    pub fn encode(&self) -> io::Result<Vec<u8>> {
        let (kind, payload) = match &self.action {
            ActivationAction::NewWindow(cwd) => (0, validate_cwd(cwd)?.as_bytes().to_vec()),
            ActivationAction::Pane(id) if *id != 0 => (1, id.to_be_bytes().to_vec()),
            _ => return Err(invalid()),
        };
        let mut out = Vec::with_capacity(49 + payload.len());
        out.extend_from_slice(b"LTA1");
        out.extend_from_slice(&self.id);
        out.extend_from_slice(&self.session);
        out.extend_from_slice(&self.expires_ns.to_be_bytes());
        out.push(kind);
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        out.extend_from_slice(&payload);
        Ok(out)
    }

    pub fn decode(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() < 49 || bytes.len() > MAX_FRAME || &bytes[..4] != b"LTA1" {
            return Err(invalid());
        }
        let len = u32::from_be_bytes(bytes[45..49].try_into().map_err(|_| invalid())?) as usize;
        if len != bytes.len() - 49 {
            return Err(invalid());
        }
        let action = match bytes[44] {
            0 => ActivationAction::NewWindow(
                validate_cwd(std::str::from_utf8(&bytes[49..]).map_err(|_| invalid())?)?.to_owned(),
            ),
            1 if len == 8 => {
                let id = u64::from_be_bytes(bytes[49..].try_into().map_err(|_| invalid())?);
                if id == 0 {
                    return Err(invalid());
                }
                ActivationAction::Pane(id)
            }
            _ => return Err(invalid()),
        };
        Ok(Self {
            id: bytes[4..20].try_into().map_err(|_| invalid())?,
            session: bytes[20..36].try_into().map_err(|_| invalid())?,
            expires_ns: u64::from_be_bytes(bytes[36..44].try_into().map_err(|_| invalid())?),
            action,
        })
    }
}

pub fn write_frame(mut out: impl Write, bytes: &[u8]) -> io::Result<()> {
    if bytes.len() > MAX_FRAME {
        return Err(invalid());
    }
    out.write_all(&(bytes.len() as u32).to_be_bytes())?;
    out.write_all(bytes)
}

pub fn read_frame(mut input: impl Read) -> io::Result<Vec<u8>> {
    let mut header = [0; 4];
    input.read_exact(&mut header)?;
    let size = u32::from_be_bytes(header) as usize;
    if size > MAX_FRAME {
        return Err(invalid());
    }
    let mut bytes = vec![0; size];
    input.read_exact(&mut bytes)?;
    Ok(bytes)
}

/// Entries survive their entire replay horizon. Saturation rejects admission;
/// an old ID can never be evicted while its original request remains valid.
#[derive(Default)]
pub struct Outcomes(Vec<(ActivationRequest, ActivationReply)>);
impl Outcomes {
    pub fn lookup(
        &mut self,
        req: &ActivationRequest,
        now: u64,
    ) -> io::Result<Option<ActivationReply>> {
        self.0.retain(|(r, _)| r.expires_ns > now);
        if req.expires_ns <= now || req.expires_ns - now > TIMEOUT.as_nanos() as u64 {
            return Err(invalid());
        }
        if let Some((old, result)) = self.0.iter().find(|(r, _)| r.id == req.id) {
            return if old == req {
                Ok(Some(result.clone()))
            } else {
                Err(invalid())
            };
        }
        if self.0.len() >= 64 {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "activation replay cache full",
            ));
        }
        Ok(None)
    }
    pub fn record(&mut self, req: ActivationRequest, reply: ActivationReply) {
        self.0.push((req, reply));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(action: ActivationAction) -> ActivationRequest {
        ActivationRequest {
            id: [1; 16],
            session: [2; 16],
            expires_ns: 200,
            action,
        }
    }
    #[test]
    fn wire_roundtrips_and_rejects_malformed_frames() {
        for action in [
            ActivationAction::NewWindow("/tmp/tiếng việt".into()),
            ActivationAction::Pane(42),
        ] {
            let r = request(action);
            let encoded = r.encode().unwrap();
            assert_eq!(ActivationRequest::decode(&encoded).unwrap(), r);
            for cut in 0..encoded.len() {
                assert!(ActivationRequest::decode(&encoded[..cut]).is_err());
            }
            let mut extra = encoded.clone();
            extra.push(0);
            assert!(ActivationRequest::decode(&extra).is_err());
            let mut version = encoded;
            version[3] = b'2';
            assert!(ActivationRequest::decode(&version).is_err());
        }
        assert!(ActivationRequest::decode(&vec![0; MAX_FRAME + 1]).is_err());
        assert!(request(ActivationAction::Pane(0)).encode().is_err());
    }
    #[test]
    fn cwd_boundary_preserves_unicode_and_rejects_invalid_paths() {
        for cwd in [
            "relative",
            "",
            "/tmp/\0bad",
            &format!("/{}", "x".repeat(MAX_CWD)),
        ] {
            assert!(validate_cwd(cwd).is_err());
        }
        assert_eq!(validate_cwd("/tmp/tiếng việt").unwrap(), "/tmp/tiếng việt");
    }
    #[test]
    fn replay_cache_never_evicts_a_valid_id() {
        let mut cache = Outcomes::default();
        let r = request(ActivationAction::Pane(1));
        assert_eq!(cache.lookup(&r, 100).unwrap(), None);
        cache.record(r.clone(), Ok(()));
        assert_eq!(cache.lookup(&r, 100).unwrap(), Some(Ok(())));
        let changed = request(ActivationAction::Pane(2));
        assert!(cache.lookup(&changed, 100).is_err());
        for i in 2..=64 {
            let mut next = r.clone();
            next.id[0] = i;
            cache.record(next, Ok(()));
        }
        let mut overflow = r.clone();
        overflow.id[0] = 65;
        assert_eq!(
            cache.lookup(&overflow, 100).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        assert_eq!(cache.lookup(&r, 100).unwrap(), Some(Ok(())));
        assert!(cache.lookup(&r, 200).is_err());
        overflow.expires_ns = 300;
        assert_eq!(cache.lookup(&overflow, 200).unwrap(), None);
    }
    #[test]
    fn framed_io_bounds_allocations_and_detects_truncation() {
        let mut bytes = Vec::new();
        write_frame(&mut bytes, b"ok").unwrap();
        assert_eq!(read_frame(bytes.as_slice()).unwrap(), b"ok");
        assert!(read_frame(&bytes[..bytes.len() - 1]).is_err());
        assert!(read_frame(((MAX_FRAME + 1) as u32).to_be_bytes().as_slice()).is_err());
    }
}
```

## Appendix — macOS Transport Reference Implementation

New `src/platform/macos/activation.rs`. Review platform wiring and add the specified
error/timeout tests before integration; capture process start before prewarm.

```rust
//! Private same-user activation sockets; one blocking worker, asleep while idle.
use crate::platform::activation::{
    self as wire, ActivationAction, ActivationRequest, ActivationSink, Outcomes, TIMEOUT,
};
use std::{
    fs,
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            fs::{DirBuilderExt, FileTypeExt, MetadataExt},
            net::{UnixListener, UnixStream},
        },
    },
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const MAX_INSTANCES: usize = 64;

pub fn monotonic_ns() -> io::Result<u64> {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: clock_gettime writes one initialized timespec; monotonic is boot-relative.
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) } != 0 {
        return Err(io::Error::last_os_error());
    }
    u64::try_from(time.tv_sec)
        .ok()
        .and_then(|s| s.checked_mul(1_000_000_000))
        .and_then(|s| s.checked_add(time.tv_nsec as u64))
        .ok_or_else(wire::invalid)
}
fn nonce() -> [u8; 16] {
    let mut bytes = [0; 16];
    // SAFETY: arc4random_buf fills exactly the writable bytes passed here.
    unsafe { libc::arc4random_buf(bytes.as_mut_ptr().cast(), bytes.len()) };
    bytes
}
fn uid() -> u32 {
    unsafe { libc::geteuid() }
}
fn hex(bytes: &[u8; 16]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn unhex(s: &str) -> io::Result<[u8; 16]> {
    if s.len() != 32 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(wire::invalid());
    }
    let mut out = [0; 16];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).map_err(|_| wire::invalid())?;
    }
    Ok(out)
}
#[derive(Debug, Clone)]
struct Origin {
    name: String,
    started: u64,
    session: [u8; 16],
}
impl Origin {
    fn parse(name: &str) -> io::Result<Self> {
        let pieces: Vec<_> = name.split('-').collect();
        if pieces.len() != 3
            || pieces[0].len() != 16
            || pieces[1].is_empty()
            || pieces[1].len() > 8
            || !pieces[0]
                .bytes()
                .chain(pieces[1].bytes())
                .all(|b| b.is_ascii_hexdigit())
        {
            return Err(wire::invalid());
        }
        u32::from_str_radix(pieces[1], 16).map_err(|_| wire::invalid())?;
        Ok(Self {
            name: name.into(),
            started: u64::from_str_radix(pieces[0], 16).map_err(|_| wire::invalid())?,
            session: unhex(pieces[2])?,
        })
    }
    fn new() -> io::Result<Self> {
        Self::parse(&format!(
            "{:016x}-{:x}-{}",
            monotonic_ns()?,
            std::process::id(),
            hex(&nonce())
        ))
    }
}
fn private_dir(path: &Path, create: bool) -> io::Result<()> {
    if create {
        match fs::DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() || meta.uid() != uid() || meta.mode() & 0o777 != 0o700 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "unsafe activation directory",
        ));
    }
    Ok(())
}
fn root(create: bool) -> io::Result<PathBuf> {
    let path = PathBuf::from(format!("/tmp/litty-a1-{}", uid()));
    private_dir(&path, create)?;
    Ok(path)
}
fn peer(stream: &UnixStream) -> io::Result<()> {
    let (mut user, mut group) = (0, 0);
    // SAFETY: getpeereid writes the uid/gid for the connected Unix socket.
    if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut user, &mut group) } != 0 {
        return Err(io::Error::last_os_error());
    }
    if user != uid() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "activation peer owner mismatch",
        ));
    }
    Ok(())
}
fn remaining(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "activation deadline expired; request may have been delivered",
            )
        })
}
struct DeadlineStream {
    stream: UnixStream,
    deadline: Instant,
}
impl Read for DeadlineStream {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.stream
            .set_read_timeout(Some(remaining(self.deadline)?))?;
        self.stream.read(bytes)
    }
}
impl Write for DeadlineStream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.stream
            .set_write_timeout(Some(remaining(self.deadline)?))?;
        self.stream.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}
fn poll_ready(fds: &mut [libc::pollfd], timeout: i32) -> io::Result<()> {
    loop {
        // SAFETY: all pollfd values refer to descriptors owned across this call.
        let rc = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout) };
        if rc > 0 {
            return Ok(());
        }
        if rc == 0 {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "activation connection timed out",
            ));
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}
fn connect(path: &Path) -> io::Result<DeadlineStream> {
    use std::os::unix::ffi::OsStrExt;
    let bytes = path.as_os_str().as_bytes();
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if bytes.len() >= address.sun_path.len() {
        return Err(wire::invalid());
    }
    address.sun_family = libc::AF_UNIX as _;
    address.sun_len = std::mem::size_of::<libc::sockaddr_un>() as u8;
    for (dst, src) in address.sun_path.iter_mut().zip(bytes) {
        *dst = *src as _;
    }
    let deadline = Instant::now() + TIMEOUT;
    // SAFETY: no pointers; returns a newly owned descriptor on success.
    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let stream = unsafe { UnixStream::from_raw_fd(fd) };
    if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    stream.set_nonblocking(true)?;
    let rc = unsafe {
        libc::connect(
            fd,
            (&address as *const libc::sockaddr_un).cast(),
            std::mem::size_of_val(&address) as _,
        )
    };
    if rc != 0 {
        let e = io::Error::last_os_error();
        if e.raw_os_error() != Some(libc::EINPROGRESS) && e.kind() != io::ErrorKind::WouldBlock {
            return Err(e);
        }
        let mut fds = [libc::pollfd {
            fd,
            events: libc::POLLOUT,
            revents: 0,
        }];
        poll_ready(
            &mut fds,
            remaining(deadline)?.as_millis().min(i32::MAX as u128) as i32,
        )?;
        if let Some(e) = stream.take_error()? {
            return Err(e);
        }
    }
    stream.set_nonblocking(false)?;
    peer(&stream)?;
    Ok(DeadlineStream { stream, deadline })
}
fn send(root: &Path, origin: &Origin, action: ActivationAction) -> io::Result<()> {
    private_dir(&root.join(&origin.name), false)?;
    let path = root.join(&origin.name).join("s");
    let meta = fs::symlink_metadata(&path)?;
    if !meta.file_type().is_socket() || meta.uid() != uid() {
        return Err(wire::invalid());
    }
    let mut stream = connect(&path)?;
    let req = ActivationRequest {
        id: nonce(),
        session: origin.session,
        expires_ns: monotonic_ns()?
            .checked_add(TIMEOUT.as_nanos() as u64)
            .ok_or_else(wire::invalid)?,
        action,
    };
    wire::write_frame(&mut stream, &req.encode()?)?;
    match wire::read_frame(&mut stream)?.as_slice() {
        [0] => Ok(()),
        [1] => Err(io::Error::other("activation request rejected")),
        _ => Err(wire::invalid()),
    }
}
fn serve_one(
    stream: UnixStream,
    session: [u8; 16],
    sink: &ActivationSink,
    outcomes: &mut Outcomes,
) -> io::Result<()> {
    peer(&stream)?;
    let mut stream = DeadlineStream {
        stream,
        deadline: Instant::now() + TIMEOUT,
    };
    let req = ActivationRequest::decode(&wire::read_frame(&mut stream)?)?;
    let result = (|| {
        if req.session != session {
            return Err(wire::invalid());
        }
        if let Some(reply) = outcomes.lookup(&req, monotonic_ns()?)? {
            return Ok(reply);
        }
        let (reply, rx) = mpsc::sync_channel(1);
        sink(req.clone(), reply);
        let result = rx
            .recv_timeout(remaining(stream.deadline)?)
            .unwrap_or_else(|_| Err("activation response unavailable".into()));
        outcomes.record(req, result.clone());
        Ok(result)
    })();
    wire::write_frame(&mut stream, &[u8::from(!matches!(result, Ok(Ok(()))))])
}

pub struct ActivationServer {
    origin: Origin,
    root: PathBuf,
    stop: UnixStream,
    worker: Option<JoinHandle<()>>,
}
impl ActivationServer {
    pub fn origin(&self) -> &str {
        &self.origin.name
    }
    pub fn session(&self) -> [u8; 16] {
        self.origin.session
    }
}
impl Drop for ActivationServer {
    fn drop(&mut self) {
        if let Err(e) = self.stop.write_all(&[1]) {
            eprintln!("litty: activation shutdown signal failed: {}", e.kind());
        }
        if self.worker.take().is_some_and(|w| w.join().is_err()) {
            eprintln!("litty: activation worker panicked");
        }
        let dir = self.root.join(&self.origin.name);
        if let Err(e) = fs::remove_file(dir.join("s")) {
            eprintln!("litty: activation socket cleanup failed: {}", e.kind());
        }
        if let Err(e) = fs::remove_dir(dir) {
            eprintln!("litty: activation directory cleanup failed: {}", e.kind());
        }
    }
}
pub fn start_activation(sink: ActivationSink) -> io::Result<ActivationServer> {
    start_at(root(true)?, sink)
}
fn start_at(root: PathBuf, sink: ActivationSink) -> io::Result<ActivationServer> {
    let origin = Origin::new()?;
    let directory = root.join(&origin.name);
    private_dir(&directory, true)?;
    let setup = (|| {
        let listener = UnixListener::bind(directory.join("s"))?;
        listener.set_nonblocking(true)?;
        let (stop, wake) = UnixStream::pair()?;
        let session = origin.session;
        let worker = thread::Builder::new()
            .name("activation".into())
            .stack_size(128 * 1024)
            .spawn(move || {
                let mut outcomes = Outcomes::default();
                loop {
                    let mut fds = [
                        libc::pollfd {
                            fd: listener.as_raw_fd(),
                            events: libc::POLLIN,
                            revents: 0,
                        },
                        libc::pollfd {
                            fd: wake.as_raw_fd(),
                            events: libc::POLLIN,
                            revents: 0,
                        },
                    ];
                    if let Err(e) = poll_ready(&mut fds, -1) {
                        eprintln!("litty: activation listener failed: {}", e.kind());
                        break;
                    }
                    if fds[1].revents != 0 {
                        break;
                    }
                    if fds[0].revents & libc::POLLIN == 0 {
                        eprintln!("litty: activation listener closed");
                        break;
                    }
                    match listener.accept() {
                        Ok((stream, _)) => {
                            if let Err(e) = serve_one(stream, session, &sink, &mut outcomes) {
                                eprintln!("litty: activation request failed: {}", e.kind());
                            }
                        }
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                        Err(e) => {
                            eprintln!("litty: activation accept failed: {}", e.kind());
                            break;
                        }
                    }
                }
            })?;
        Ok((stop, worker))
    })();
    match setup {
        Ok((stop, worker)) => Ok(ActivationServer {
            origin,
            root,
            stop,
            worker: Some(worker),
        }),
        Err(e) => {
            match fs::remove_file(directory.join("s")) {
                Ok(()) => {}
                Err(clean) if clean.kind() == io::ErrorKind::NotFound => {}
                Err(clean) => eprintln!("litty: activation setup cleanup failed: {}", clean.kind()),
            }
            if let Err(clean) = fs::remove_dir(directory) {
                eprintln!("litty: activation setup cleanup failed: {}", clean.kind());
            }
            Err(e)
        }
    }
}
fn reuse_at(root: &Path, cwd: &str) -> io::Result<bool> {
    wire::validate_cwd(cwd)?;
    let mut origins = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if let Some(name) = entry.file_name().to_str() {
            if let Ok(origin) = Origin::parse(name) {
                if origins.len() >= MAX_INSTANCES {
                    return Err(io::Error::other("too many activation instances"));
                }
                origins.push(origin);
            }
        }
    }
    origins.sort_by(|a, b| b.started.cmp(&a.started).then(b.name.cmp(&a.name)));
    for origin in origins {
        match send(root, &origin, ActivationAction::NewWindow(cwd.into())) {
            Ok(()) => return Ok(true),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                ) =>
            {
                continue;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(false)
}
pub fn reuse_window(cwd: &str) -> io::Result<bool> {
    wire::validate_cwd(cwd)?;
    match root(false) {
        Ok(dir) => reuse_at(&dir, cwd),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}
pub fn forward_notification(origin: &str, pane: u64) -> io::Result<()> {
    send(
        &root(false)?,
        &Origin::parse(origin)?,
        ActivationAction::Pane(pane),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let p = PathBuf::from(format!("/tmp/la-test-{}", hex(&nonce())));
            private_dir(&p, true).unwrap();
            Self(p)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir(&self.0).unwrap();
        }
    }
    #[test]
    fn real_transport_selects_latest_and_cleans_up() {
        let f = Fixture::new();
        assert!(!reuse_at(&f.0, "/tmp").unwrap());
        let (tx, rx) = mpsc::sync_channel(4);
        let first = tx.clone();
        let a = start_at(
            f.0.clone(),
            Arc::new(move |req, reply| {
                first.send((1, req.action)).unwrap();
                reply.send(Ok(())).unwrap();
            }),
        )
        .unwrap();
        let b = start_at(
            f.0.clone(),
            Arc::new(move |req, reply| {
                tx.send((2, req.action)).unwrap();
                reply.send(Ok(())).unwrap();
            }),
        )
        .unwrap();
        assert!(b.origin.started >= a.origin.started);
        assert!(reuse_at(&f.0, "/tmp/tiếng việt").unwrap());
        assert_eq!(
            rx.recv_timeout(TIMEOUT).unwrap(),
            (2, ActivationAction::NewWindow("/tmp/tiếng việt".into()))
        );
        drop(b);
        assert!(reuse_at(&f.0, "/tmp").unwrap());
        assert_eq!(rx.recv_timeout(TIMEOUT).unwrap().0, 1);
        drop(a);
        assert_eq!(fs::read_dir(&f.0).unwrap().count(), 0);
    }
    #[test]
    fn origin_parser_rejects_paths_and_bad_session_identifiers() {
        for s in ["../../tmp", "", "0-1-aa", "0000000000000001-1-🙂"] {
            assert!(Origin::parse(s).is_err());
        }
        let origin = Origin::new().unwrap();
        assert_eq!(Origin::parse(&origin.name).unwrap().session, origin.session);
    }
    #[test]
    fn unsafe_directories_and_symlinks_are_rejected() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let f = Fixture::new();
        let link = f.0.join("link");
        symlink(&f.0, &link).unwrap();
        assert!(private_dir(&link, false).is_err());
        fs::remove_file(link).unwrap();
        fs::set_permissions(&f.0, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            private_dir(&f.0, false).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
    }
    #[test]
    fn rejected_delivery_is_an_error_instead_of_fallback() {
        let f = Fixture::new();
        let server = start_at(
            f.0.clone(),
            Arc::new(|_, reply| reply.send(Err("failed".into())).unwrap()),
        )
        .unwrap();
        assert!(reuse_at(&f.0, "/tmp").is_err());
        drop(server);
    }
}
```

## Execution Checkpoint — 2026-11-15 (in progress)

- Plan/probe committed as `ec69968`; implementation remains uncommitted on
  `feat/app-activation` at the maintainer's request to proceed.
- Common codec/path/replay tests: 4 red with stubs, then 4 green.
- macOS transport tests: 2 red with start stub, then 4 green outside sandbox.
  Unix socket creation is sandbox-restricted. Separately, standard
  `UnixStream::set_read_timeout` returns EINVAL for a simple socket pair even
  outside sandbox on this host. Nonblocking I/O plus existing `poll` deadlines
  replaces socket timeout options; no behavior/interface change is intended.
- Initial strict PTY tests: missing method/argument red, then full PTY module
  17 passed outside sandbox, including nonexistent shell/error propagation.
- Current macOS `cargo check` and Linux x86_64 cross-check passed with zero
  warnings after removing notification-only APIs until native gate opens.
- CLI reuse variant, ordinary-instance service, initial cwd and exact-pane app
  handler are wired. Full CLI/app/GUI acceptance is not yet verified.
- Passive activation regression reproduces Reopen emission on the unchanged
  base callback; callback now avoids generic Reopen. Green rerun is pending.
- Supplemental interim review found three issues to fix with regressions:
  low descriptor numbers can overwrite strict PTY error pipes; timed-out
  requests can accumulate queued app events; crash tombstones can fill the
  instance-discovery cap. Do not mark these tasks complete before fixes.
- Notification probe authorization returned UNErrorDomain=1. A question asking
  permission to enable the test probe's notifications is pending. No native
  delivery/click/cold-start implementation or acceptance is claimed.
- Remaining gates: strict full-code native plan appendix and Opus reviewer are
  unavailable/unfulfilled, as described above; final independent supplemental
  review, full suite, Clippy/builds and real app flow still must run.

## Execution Checkpoint 2 — Reuse Acceptance

- Real release GUI/PTY/CLI harness passes no-instance startup, Unicode/spaced
  directory handoff, normal independent launch, newest eligible selection,
  stopped-process timeout without fallback, crash cleanup and zero-window reuse.
- Closed-window sample: 29 MB physical footprint; CPU time 0:00.23 unchanged
  across at least five seconds. See the [partial validation report](../validation/macos-app-activation-2026-11-15.md).
- Interim review findings were reproduced and fixed: descriptors above stdio
  for both setup pipes and PTYs; interrupted/short errno writes; one admission
  held through app consumption after timeout; verified-dead private endpoint
  pruning before bounded discovery. Registry scanning now caps at 4096 entries
  rather than counting 64 crash tombstones as eligible instances.
- macOS/Linux checks and Clippy pass with zero warnings. The first complete
  outside-sandbox suite passed 690 tests total, 12 existing ignored. The final
  rerun after added error-write/closed-stdio assertions is in progress.
- Native notification Task 4 is still blocked by the authorization/launch
  characterization gate. The permission question remains pending.

## Execution Checkpoint 3 — Final Partial Handoff

- Latest source passes `cargo check`, Linux x86_64 check, Apple x86_64 check,
  Clippy with denied warnings, release build and full outside-sandbox tests:
  679 unit + 1 + 14 integration tests passed (694 total), 12 existing ignored.
  Python harness safety suite: 2 passed. No new ignored tests.
- Release CLI/GPU/PTY acceptance was rerun after the fixes and passes. The
  guarded harness refuses populated/unsafe registries; do not launch other
  Litty instances concurrently with the test.
- Latest closed-window footprint is 30 MB. CPU time increased 0.01 s over at
  least five seconds; 0% idle CPU is not established by that sample. The earlier
  29 MB sample had flat CPU time. No unrelated cause is asserted.
- Review round 1 corrections are implemented with red/green regressions:
  idempotent concurrent endpoint deletion, private monotonic helper, safe
  harness preflight, and captured-directory rename/removal/search permissions.
  APFS resolved-path identity is checked because its removed directory can
  retain a nonzero link count. See the partial validation report for details.
- Code remains uncommitted; only planning/probe commits were requested. No
  implementation merge, push or release is authorized or performed.
- Native notification delivery/routing/cold-start remains unimplemented. The
  notification permission request is still pending; the temporary probe was
  stopped after confirming its identity, rather than left running indefinitely.
- Strict Opus review and full literal native implementation planning remain
  unfulfilled/unavailable. Do not report the whole feature or plan complete.

The appendices above preserve the initial reference implementations, not a
patch to reapply over the current source. Current source and these execution
checkpoints supersede their socket-timeout, cleanup and API-visibility details.

## Execution Checkpoint 4 — Supplemental Review Complete

- Round 2 platform, app/PTY and docs/harness reviews approve the partial reuse
  scope with no remaining Critical/Important findings in that scope. All
  round-1 fixable findings were verified fixed; none were rejected.
- The approval is supplemental only. Required Opus review/full native planning
  remain unfulfilled. Native notifications, older OS runtime/cross-user checks
  and exact 0% idle CPU remain open, as the validation report records.
- Native notification work is awaiting explicit permission to enable the
  local test probe's notifications. Repeated continue requests resume work;
  they have not been treated as consent to change system settings.
- No source edits followed the 694-test verification and final release
  acceptance. Only review-status documentation was appended.

## Notification Permission Follow-up

The maintainer explicitly authorized enabling notifications for the local probe.
The attempt to open System Settings through computer use failed with
`Computer Use permissions are not granted`. No setting was changed. This is
a separate tool-access blocker from the probe's notification authorization
error; the earlier request for permission is now answered. Native notification
characterization remains incomplete until computer control is enabled or the
maintainer enables the probe's notification setting manually.

## Execution Checkpoint 5 — Notification Permission Granted

The maintainer reports allowing notifications; process 26938's log confirms
authorization and delivery success. Live-click/cold-start characterization can
now proceed on the registered project-directory probe. Earlier permission
blocker notes are historical, superseded by this observation.

## Execution Checkpoint 6 — Cold Launch Characterized

- Process 26938 live default-action callback was followed by becameActive and
  reopen. After the probe processes exited, a click cold-launched process
  34052. Before the response callback, didFinish contained default=0 and
  NSApplicationLaunchUserNotificationKey whose class was UNNotificationResponse.
- This observed modern UN payload supports startup classification before PTY
  creation; no timer or legacy-class assumption is needed. The earlier launch
  gate is resolved for this machine/bundle. Older macOS runtime remains to test.
- Native implementation now in progress: early delegate setup, bounded exact
  origin routing, native delivery and generic-reopen suppression. Cold-start
  policy regression failed first (would create shell) before production logic.
  All acceptance/checks must rerun after these source changes.

## Checkpoint Commit Authorized

The maintainer requested committing the current implementation before continuing.
This checkpoint includes reuse and native notification groundwork; the native
reopen ordering finding remains open. Latest full suite: 685 unit + 1 + 14
integration = 700 passed, 12 existing ignored. macOS/Linux checks, Apple x86_64
check, Clippy and diff check pass. Native visual/focus acceptance remains pending.
The user-authorized checkpoint is not a declaration that the whole plan is done.
