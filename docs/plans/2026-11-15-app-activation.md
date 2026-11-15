# App Activation — Open Questions (plan blocked)

The Wayland preview phase is complete. This document scopes the separate
notification click-through and existing-instance activation follow-ups.
No implementation tasks are defined until the behavior below is confirmed.

Existing code: `src/platform/process.rs` shows notifications through
`notify-send` on Linux and `osascript` on macOS, without click callbacks.
`src/app/terminal_io.rs` drains notifications per pane but passes only title
and body to that helper. `src/app/user_event.rs::on_reopen` activates the
first existing window or creates one when none exists.
`src/platform/linux/native/wayland_activation.rs` handles launcher tokens;
the Wayland plan records that no single-instance IPC exists.

1. **Platform scope:** Linux (Wayland and X11), with an explicit macOS
   counterpart/no-op for new platform interfaces; or full Linux and macOS
   notification click-through in this phase?
   **Recommendation:** Linux first, preserving current macOS behavior.
   **Affects:** native notification integration, platform parity, desktop tests.
2. **Notification destination and stale targets:** activate the originating
   window, tab and pane, ignoring clicks after that pane closes; or activate
   the most recent window, creating a new window if necessary?
   **Recommendation:** exact originating pane; ignore stale clicks without
   creating a shell. Reveal that pane if its tab or split is hidden/zoomed.
   **Affects:** target identity, tab/split selection, close races and tests.
3. **Existing-instance launch behavior:** keep normal invocations independent
   and add an explicit reuse mode; or reuse an existing process by default?
   When reusing, open a new window or focus an existing one?
   **Recommendation:** explicit reuse mode that opens a new window in the
   existing process, preserving the invoking directory and activation token;
   normal invocations remain independent. Benchmarks and utility commands
   stay local. CLI spelling and IPC format require approval in the final plan.
   **Affects:** launcher/CLI behavior, startup ownership, IPC messages, failure
   handling and compatibility. The implementation plan must define behavior
   when the existing process is unavailable or a request times out.

Planning gate: `writing-plans-strict` requires answers to questions affecting
visible behavior or interfaces before implementation tasks are written.
