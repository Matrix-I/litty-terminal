#!/usr/bin/env bash
# Build a release binary and wrap it in the Linux desktop packages, the Linux
# counterpart of bundle_macos.sh. Packaging itself lives in scripts/linux/ and
# is shared with the release workflow.
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$DIR"

if [ $# -ne 0 ]; then
    echo "Usage: $0" >&2
    exit 1
fi

# The package scripts only accept x86_64 ELF executables.
if [ "$(uname -s)" != Linux ] || [ "$(uname -m)" != x86_64 ]; then
    echo "ERROR: $0 builds Linux x86_64 packages only." >&2
    exit 1
fi

VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)"
# Checked here, not only in the package scripts, to fail before a long build.
if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "ERROR: could not read a MAJOR.MINOR.PATCH version from Cargo.toml." >&2
    exit 1
fi
DIST_DIR="$DIR/dist"
BINARY_NAME="litty-linux-x86_64"

# The updater only installs releases signed by the key baked in at build time
# (`LITTY_MINISIGN_PUBKEY`, see src/update/verify.rs); default to the project's key.
MINISIGN_PUB_FILE="$DIR/resources/minisign.pub"
if [ -z "${LITTY_MINISIGN_PUBKEY:-}" ] && [ -f "$MINISIGN_PUB_FILE" ]; then
    LITTY_MINISIGN_PUBKEY="$(tail -n 1 "$MINISIGN_PUB_FILE")"
    export LITTY_MINISIGN_PUBKEY
fi
if [ -z "${LITTY_MINISIGN_PUBKEY:-}" ]; then
    echo "WARNING: no minisign public key; this build will refuse all auto-updates." >&2
fi

echo "==> Building optimized Linux release binary (v$VERSION)..."
if [ -f "$HOME/.cargo/env" ]; then
    . "$HOME/.cargo/env"
fi
cargo build --release
BUILT="${CARGO_TARGET_DIR:-$DIR/target}/release/litty"
if [ ! -f "$BUILT" ]; then
    # Like bundle_macos.sh, only the default layout and CARGO_TARGET_DIR are
    # supported, not build.target-dir or build.target in a cargo config.
    echo "ERROR: built binary not found at $BUILT." >&2
    exit 1
fi

mkdir -p "$DIST_DIR"
install -m 755 "$BUILT" "$DIST_DIR/$BINARY_NAME"

# Unlike the release workflow (cargo-zigbuild, glibc 2.34), this binary needs
# the build host's glibc, so the packages suit this machine and newer distros.
echo "==> Packaging $BINARY_NAME..."
OUTPUTS=("$DIST_DIR/$BINARY_NAME")
if command -v dpkg-deb >/dev/null 2>&1; then
    bash scripts/linux/package-deb.sh "$DIST_DIR/$BINARY_NAME" "$VERSION" "$DIST_DIR"
    OUTPUTS+=("$DIST_DIR/litty_${VERSION}_amd64.deb")
else
    echo "WARNING: dpkg-deb not installed; skipping the .deb package." >&2
fi
if command -v rpmbuild >/dev/null 2>&1; then
    # rpmbuild also checks BuildRequires against the rpm database, which is
    # empty on Debian-family hosts, so a failure here must not discard the .deb.
    if bash scripts/linux/package-rpm.sh "$DIST_DIR/$BINARY_NAME" "$VERSION" "$DIST_DIR"; then
        OUTPUTS+=("$DIST_DIR/litty-${VERSION}-1.x86_64.rpm")
    else
        echo "WARNING: rpmbuild failed (see above); skipping the .rpm package." >&2
    fi
else
    echo "WARNING: rpmbuild not installed; skipping the .rpm package." >&2
fi

echo "==> Done! Outputs ready:"
ls -lh "${OUTPUTS[@]}"
