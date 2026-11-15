# macOS App Activation — Open Questions (plan blocked)

The Wayland preview phase is complete. This is a separate macOS feature;
Wayland activation-token forwarding and Linux notification click-through
are outside this phase.

## Confirmed Requirements (2026-11-15)

- Platform: macOS, as explicitly selected by the maintainer. Linux keeps
  existing behavior; any new platform interface needs a matching Linux
  implementation or explicit no-op, exported by name.
- A notification click selects the exact originating window, tab and pane,
  revealing a hidden tab/split if necessary. If the pane has closed, ignore
  the click without opening a shell. The maintainer accepted recommendation 2.
- Reuse is explicit opt-in. It opens a new window in an existing process,
  preserving the invoking directory. Normal invocations remain independent;
  benchmarks and utility commands remain local. The maintainer accepted
  recommendation 3. macOS uses AppKit activation; Wayland tokens do not apply.
- Work branch: `feat/app-activation`. Planning-document commits are authorized;
  implementation, API/CLI-format changes, dependencies and publishing require
  their applicable project gates. No implementation is requested in this turn.

## Existing Code and Design Constraints

- `src/platform/process.rs` shows notifications through `osascript` on macOS
  and `notify-send` on Linux, with no click callbacks.
- `src/app/terminal_io.rs::handle_terminal_requests` drains notifications per
  pane but passes only title and body to that helper.
- `src/app/user_event.rs::on_reopen` activates the first existing window or
  creates one when none exists. Generic activation must not override the
  pane selected by a notification response.
- `src/app/window.rs::create_window` already accepts an optional directory.
- `src/platform/macos/app.rs::register_reopen_handler` installs delegate
  callbacks; `activate_app` currently calls `activateIgnoringOtherApps:`.
- `src/platform/macos/objc.rs` provides Objective-C runtime access and
  autorelease-pool management. No new dependency has been selected.
- `src/main.rs` dispatches commands parsed by `src/cli.rs`; there is no reuse
  command or existing-instance IPC. Neither ordinary launches nor benchmark
  processes may silently change ownership.
- `resources/Info.plist` identifies `com.litty.terminal` and supports macOS 11+.
  Native notification delivery must account for bundled and bare CLI launches,
  notification permission denial, and callbacks arriving after pane/process exit.
- Platform boundaries are explicit in `src/platform/mod.rs` and
  `src/platform/types.rs`; platform code cannot depend on app/terminal state.

## Remaining Behavior Question

1. **Reuse selection and failure policy:**
   - **Recommended:** if no eligible instance exists, start a new instance;
     if several exist, target the most recently started eligible instance;
     if request delivery times out, report an error without automatically
     creating another window, because the request may already have succeeded.
   - **Alternative:** require an explicitly named target instance; separately
     choose whether a missing instance is an error or starts a new one.
   - **Affects:** discovery/ownership, CLI semantics, acknowledgement and
     deduplication, process races, startup failure tests and user-visible errors.
   - **Status:** awaiting maintainer response; the three confirmed requirements
     above do not settle these edge cases.

## Official API References

- [UNUserNotificationCenterDelegate](https://developer.apple.com/documentation/UserNotifications/UNUserNotificationCenterDelegate)
  handles notification responses. The implementation plan must specify delegate
  lifetime, completion-handler ownership, main-thread event delivery and tests.
- [AppKit macOS 14 release notes](https://developer.apple.com/documentation/macos-release-notes/appkit-release-notes-for-macos-14)
  deprecate `activateIgnoringOtherApps:` and describe cooperative activation.
  Any activation code touched must use modern APIs where available, retain a
  macOS 11-compatible availability path, and respect OS activation policy.

## Planning Gate

`writing-plans-strict` requires answers to questions affecting visible behavior
or interfaces before implementation tasks are written. No tasks are defined yet.
After the remaining answer, expand this document into test-first tasks and run
its required plan review. This scope record is not an approved implementation plan.
