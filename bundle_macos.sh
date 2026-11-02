#!/bin/bash
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$DIR"

INSTALL=true
case "${1:-}" in
    --no-install) INSTALL=false ;;
    "") ;;
    *) echo "Usage: $0 [--no-install]" >&2; exit 1 ;;
esac

APP_NAME="Litty.app"
DIST_DIR="$DIR/dist"
APP_DIR="$DIST_DIR/$APP_NAME"
CONTENTS="$APP_DIR/Contents"
MACOS="$CONTENTS/MacOS"
RESOURCES="$CONTENTS/Resources"

# The updater only installs releases signed by the key baked in at build time
# (`LITTY_MINISIGN_PUBKEY`, see src/update.rs); default to the project's key.
MINISIGN_PUB_FILE="$DIR/resources/minisign.pub"
MINISIGN_SEC_FILE="${MINISIGN_SECRET_KEY:-$HOME/.minisign/minisign.key}"
if [ -z "${LITTY_MINISIGN_PUBKEY:-}" ] && [ -f "$MINISIGN_PUB_FILE" ]; then
    LITTY_MINISIGN_PUBKEY="$(tail -n 1 "$MINISIGN_PUB_FILE")"
    export LITTY_MINISIGN_PUBKEY
fi
if [ -z "${LITTY_MINISIGN_PUBKEY:-}" ]; then
    echo "WARNING: no minisign public key; this build will refuse all auto-updates." >&2
fi

echo "==> Building optimized macOS release binary..."
if [ -z "${SDKROOT:-}" ]; then
    export SDKROOT="$(xcrun --show-sdk-path 2>/dev/null || true)"
fi
if [ -f "$HOME/.cargo/env" ]; then
    . "$HOME/.cargo/env"
fi
cargo build --release


echo "==> Packaging $APP_NAME..."
rm -rf "$APP_DIR"
mkdir -p "$MACOS" "$RESOURCES"

# Copy binary
cp "target/release/litty" "$MACOS/litty"
chmod +x "$MACOS/litty"

# Copy Info.plist and Icon
cp "resources/Info.plist" "$CONTENTS/Info.plist"
cp "resources/Litty.icns" "$RESOURCES/Litty.icns"

# Sign ad-hoc so macOS Gatekeeper allows immediate execution without quarantine warnings
codesign --force --deep --sign - "$APP_DIR"
touch "$APP_DIR"

echo "==> $APP_NAME created at $APP_DIR"

if [ "$INSTALL" = true ] && [ -d "$HOME/Applications" ]; then
    echo "==> Installing to $HOME/Applications/$APP_NAME..."
    rm -rf "$HOME/Applications/$APP_NAME" 2>/dev/null || true
    cp -R "$APP_DIR" "$HOME/Applications/" 2>/dev/null || true
    codesign --force --deep --sign - "$HOME/Applications/$APP_NAME" 2>/dev/null || true
    touch "$HOME/Applications/$APP_NAME" 2>/dev/null || true
fi

# Also package a convenient drag-and-drop DMG for distribution
DMG_NAME="Litty.dmg"
DMG_PATH="$DIST_DIR/$DMG_NAME"
DMG_STAGING="$DIST_DIR/dmg_staging"

echo "==> Creating installable disk image ($DMG_NAME)..."
rm -rf "$DMG_STAGING" "$DMG_PATH"
mkdir -p "$DMG_STAGING"

cp -R "$APP_DIR" "$DMG_STAGING/"
ln -s /Applications "$DMG_STAGING/Applications"

# `hdiutil create` is deprecated in favour of `diskutil image`; older macOS
# releases ship without the latter, so keep hdiutil as the fallback there.
if diskutil image create from --help >/dev/null 2>&1; then
    diskutil image create from --format UDZO --volumeName "Litty" \
        "$DMG_STAGING" "$DMG_PATH"
else
    hdiutil create -volname "Litty" \
        -srcfolder "$DMG_STAGING" \
        -ov -format UDZO \
        "$DMG_PATH"
fi

rm -rf "$DMG_STAGING"

echo "==> Creating standalone binary and zip archive..."
cp "target/release/litty" "$DIST_DIR/litty-macos"
chmod +x "$DIST_DIR/litty-macos"
(cd "$DIST_DIR" && rm -f Litty-macos.zip && zip -q -r Litty-macos.zip "$APP_NAME")

# Architecture-suffixed copies (Rust `std::env::consts::ARCH` naming) for the self-updater.
ARCH="$(uname -m)"
[ "$ARCH" = "arm64" ] && ARCH="aarch64"
cp "$DIST_DIR/litty-macos" "$DIST_DIR/litty-macos-$ARCH"
cp "$DIST_DIR/Litty-macos.zip" "$DIST_DIR/Litty-macos-$ARCH.zip"

echo "==> Writing checksums and minisign signature..."
SUMS="$DIST_DIR/SHA256SUMS.txt"
rm -f "$SUMS" "$SUMS.minisig"
(cd "$DIST_DIR" && shasum -a 256 "$DMG_NAME" "Litty-macos-$ARCH.zip" "litty-macos-$ARCH" > SHA256SUMS.txt)
if ! command -v minisign >/dev/null 2>&1; then
    echo "WARNING: minisign not installed (brew install minisign); $SUMS left unsigned." >&2
elif [ ! -f "$MINISIGN_SEC_FILE" ]; then
    echo "WARNING: no minisign secret key at $MINISIGN_SEC_FILE; $SUMS left unsigned." >&2
else
    minisign -S -s "$MINISIGN_SEC_FILE" -m "$SUMS"
    # Catch a secret key that does not match the key baked into the binary.
    if [ -n "${LITTY_MINISIGN_PUBKEY:-}" ]; then
        if ! minisign -V -q -m "$SUMS" -P "$LITTY_MINISIGN_PUBKEY"; then
            rm -f "$SUMS.minisig"
            echo "ERROR: $MINISIGN_SEC_FILE does not match LITTY_MINISIGN_PUBKEY." >&2
            exit 1
        fi
    fi
fi

echo "==> Done! Outputs ready:"
ls -lh "$APP_DIR"
ls -lh "$DMG_PATH"
ls -lh "$DIST_DIR/litty-macos"
ls -lh "$DIST_DIR/Litty-macos.zip"
ls -lh "$SUMS"*
