# Desktop notifications

Native macOS notification integration is merged to `main` (PR #83); desktop
acceptance is still in progress. See the
[validation report](../validation/macos-app-activation-2026-11-15.md) for tested
coverage and open checks. It is not claimed as a completed released feature.

Programs running inside Litty can request a notification using OSC 9 or OSC 777:

```sh
printf '\033]9;Task finished\007'
printf '\033]777;notify;Build finished;The build completed successfully\007'
```

## macOS app builds

Run the packaged `Litty.app` for native notifications under Litty's identity.
The first notification requests permission; allow it to enable delivery. If
permission is denied or unavailable (for example, a relocated or re-signed
bundle), Litty shows the notification through the standalone helper instead:
it still appears, but without Litty's identity or click routing. After
choosing Don't Allow, notifications therefore keep appearing under the
helper's identity; turn them off in System Settings ▸ Notifications for that
helper (Script Editor) if you do not want them. Delivery, single-process
click focus and cold-launch clicks have been verified under the real
`com.litty.terminal` identity. The app bundle supplies the notification
icon through its `Litty.icns` resource.
Setting the terminal window title or Dock icon does not replace that resource.

Click routing targets the originating live pane, including its current tab
and window after a tab move. While Litty is running, a closed pane is ignored
without creating a replacement window. If Litty has fully exited and an old
notification relaunches it, Litty opens one fresh window using its normal
startup directory. It does not restore the exited pane or its shell session.
Clean quits withdraw that process's notifications, both through Cmd+Q and
through quit requests from macOS such as Dock Quit. An old notification may
remain after a crash.

With several Litty processes running, macOS may hand a click to a process
other than its origin. Litty forwards it, so the right pane is selected, but
on macOS 14 and later the origin's window may stay behind other apps. Keep
one installed copy of Litty.app: duplicate copies registered with the same
bundle identifier can make macOS launch or activate the wrong copy. See the
validation report for the maintainer's completed UI checks and the remaining
runtime/review limits.

Ordinary launches remain independent. Notification clicks route to their exact
origin rather than the newest process selected by `+reuse`.

## Standalone binaries and Linux

A macOS binary run outside an app bundle retains the existing helper delivery
path. Its notification identity/icon and click behavior do not represent the
packaged app's native integration.

Linux continues to use `notify-send`. Native notification pane click-through
is outside this phase's Linux scope.
