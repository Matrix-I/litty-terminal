#!/bin/sh
# Replays the notification add/remove race at clean shutdown with a separate
# test-only bundle: each round submits notifications, withdraws them before the
# adds are acknowledged, and exits at once (as litty_notifications_shutdown
# does). A second process then counts leftovers. A negative control round that
# skips the withdrawal must leave some, or the count proves nothing.
# Posts and removes real notifications; needs the probe identity authorized.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
rounds=${ROUNDS:-5}
count=${COUNT:-64}
# Delivery lags the exited sender by seconds; count only after it settles.
settle=${SETTLE:-10}
# Notification authorization was observed only for bundles under dist/.
mkdir -p "$root/dist/activation-test"
probe_dir=$(mktemp -d "$root/dist/activation-test/race.XXXXXX")
bundle="$probe_dir/LittyActivationProbe.app"
log="$probe_dir/events.log"
mkdir -p "$bundle/Contents/MacOS"
cat > "$bundle/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict><key>CFBundleIdentifier</key><string>com.litty.activation-probe</string><key>CFBundleExecutable</key><string>probe</string><key>CFBundleName</key><string>Litty Activation Probe</string><key>CFBundlePackageType</key><string>APPL</string><key>LSMinimumSystemVersion</key><string>11.0</string></dict></plist>
PLIST
xcrun clang "-DPROBE_LOG=\"$log\"" -fobjc-arc -mmacosx-version-min=11.0 -framework AppKit -framework UserNotifications "$root/scripts/macos/shutdown_race_probe.m" -o "$bundle/Contents/MacOS/probe"
codesign --force --sign - "$bundle"

leftover() {
    : > "$log"
    open -n -W "$bundle" --args --count --settle "$settle"
    sed -n 's/.*leftover count=\([0-9]*\).*/\1/p' "$log"
}

leftover > /dev/null # clear anything a previous run left behind
open -n -W "$bundle" --args --race control --n "$count" --no-remove 1
control=$(leftover)
echo "control (no withdrawal): leftover=$control"
if [ "${control:-0}" -eq 0 ]; then
    echo "FAIL: the control left nothing, so the count cannot detect a leak" >&2
    exit 1
fi

failed=0
i=1
while [ "$i" -le "$rounds" ]; do
    for mode in same next; do
        if [ "$mode" = next ]; then
            open -n -W "$bundle" --args --race "$mode$i" --n "$count" --next-turn 1
        else
            open -n -W "$bundle" --args --race "$mode$i" --n "$count"
        fi
        # Adds acknowledged before the withdrawal started; 0 means the
        # withdrawal really raced in-flight adds.
        acked=$(awk '/removing/{print c+0; exit} /late-completion/{c++}' "$log")
        left=$(leftover)
        echo "round $i $mode-turn: acked-before-withdraw=$acked leftover=$left"
        [ "${left:-1}" -eq 0 ] || failed=1
    done
    i=$((i + 1))
done
[ "$failed" -eq 0 ] || { echo "FAIL: a withdrawn notification survived exit" >&2; exit 1; }
echo "PASS: $((rounds * 2)) rounds x $count notifications, none survived exit"
