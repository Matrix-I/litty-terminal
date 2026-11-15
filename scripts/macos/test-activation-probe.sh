#!/bin/sh
# Build a separate test-only bundle; no terminal shell or private data is used.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
probe_dir=$(mktemp -d /tmp/litty-activation-probe.XXXXXX)
bundle="$probe_dir/LittyActivationProbe.app"
mkdir -p "$bundle/Contents/MacOS"
cat > "$bundle/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict><key>CFBundleIdentifier</key><string>com.litty.activation-probe</string><key>CFBundleExecutable</key><string>probe</string><key>CFBundleName</key><string>Litty Activation Probe</string><key>CFBundlePackageType</key><string>APPL</string><key>LSMinimumSystemVersion</key><string>11.0</string></dict></plist>
PLIST
xcrun clang "-DPROBE_LOG=\"$probe_dir/events.log\"" -fobjc-arc -mmacosx-version-min=11.0 -framework AppKit -framework UserNotifications "$root/scripts/macos/activation_probe.m" -o "$bundle/Contents/MacOS/probe"
codesign --force --sign - "$bundle"
printf 'Probe bundle: %s\nLaunch: open -n "%s" --args --notify\nEvents: %s/events.log\n' "$bundle" "$bundle" "$probe_dir"
