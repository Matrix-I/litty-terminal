# Desktop notifications

Native macOS notification integration is implemented on `feat/app-activation`;
desktop acceptance is still in progress. See the
[validation report](../validation/macos-app-activation-2026-11-15.md) for tested
coverage and open checks. It is not claimed as a completed released feature.

Programs running inside Litty can request a notification using OSC 9 or OSC 777:

```sh
printf '\033]9;Task finished\007'
printf '\033]777;notify;Build finished;The build completed successfully\007'
```

## macOS app builds

Run the packaged `Litty.app` for native notifications under Litty's identity.
The first notification requests permission; allow it to enable delivery. The
app bundle supplies the notification icon through its `Litty.icns` resource.
Setting the terminal window title or Dock icon does not replace that resource.

Click routing targets the originating live pane, including its current tab
and window after a tab move. A closed pane or exited process is ignored;
notification-only relaunch must not create a replacement shell. Those desktop
flows, including visual icon/focus and Dock interaction, remain acceptance
checks; unit and lifecycle tests do not establish all OS behavior.

Ordinary launches remain independent. Notification clicks route to their exact
origin rather than the newest process selected by `+reuse`.

## Standalone binaries and Linux

A macOS binary run outside an app bundle retains the existing helper delivery
path. Its notification identity/icon and click behavior do not represent the
packaged app's native integration.

Linux continues to use `notify-send`. Native notification pane click-through
is outside this phase's Linux scope.
