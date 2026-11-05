#!/usr/bin/env bash
# Package an existing release executable without rebuilding or changing it.
set -euo pipefail
umask 022
root=$(cd "$(dirname "$0")/../.." && pwd)
if [[ $# != 3 ]]; then
    echo "Usage: $0 BINARY VERSION OUTPUT_DIRECTORY" >&2
    exit 1
fi
binary=$1
version=$2
output=$3
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo 'VERSION must be a numeric MAJOR.MINOR.PATCH release.' >&2
    exit 1
fi
if [[ ! -f "$binary" ]]; then
    echo 'BINARY must be an existing Linux x86_64 executable.' >&2
    exit 1
fi
# Check ELF64, little endian, ET_EXEC/ET_DYN, and the x86_64 machine type.
header=$(od -An -tx1 -N20 "$binary" | tr -d ' \n')
if [[ ! "$header" =~ ^7f454c46020101[0-9a-f]{18}0[23]003e00$ ]]; then
    echo 'BINARY must be a little-endian Linux x86_64 ELF executable.' >&2
    exit 1
fi
command -v dpkg-deb >/dev/null
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage/DEBIAN" "$stage/usr/bin" "$stage/usr/share/applications" \
    "$stage/usr/share/icons/hicolor/512x512/apps" "$stage/usr/share/doc/litty"
install -m 755 "$binary" "$stage/usr/bin/litty"
install -m 644 "$root/resources/linux/litty.desktop" "$stage/usr/share/applications/litty.desktop"
install -m 644 "$root/resources/icon.png" "$stage/usr/share/icons/hicolor/512x512/apps/litty.png"
cat > "$stage/usr/share/doc/litty/README.Debian" <<'DOC'
Launch Litty Terminal from your desktop's applications menu or run /usr/bin/litty.
This package requires a working Vulkan driver and an X11 or Wayland desktop.
Install newer .deb releases with apt; the built-in updater cannot replace the
root-owned /usr/bin/litty as an ordinary user. Do not run Litty as root to update.
A previous per-user ~/.local/share/applications/litty.desktop overrides the
packaged launcher. Remove that old manual shortcut if it points elsewhere.
DOC
installed_size=$(du -sk "$stage/usr" | awk '{print $1}')
cat > "$stage/DEBIAN/control" <<CONTROL
Package: litty
Version: $version
Section: x11
Priority: optional
Architecture: amd64
Maintainer: Litty contributors <noreply@github.com>
Installed-Size: $installed_size
Depends: libc6 (>= 2.34), libgcc-s1, libfontconfig1, libxkbcommon0, libxkbcommon-x11-0, libvulkan1, libwayland-client0, libwayland-cursor0, libwayland-egl1, libx11-6, libx11-xcb1, libxcb1, libxcursor1, libxi6, libxrandr2
Recommends: mesa-vulkan-drivers | vulkan-icd, fonts-dejavu-core, xdg-utils, wl-clipboard | xclip | xsel
Homepage: https://github.com/Matrix-I/litty-terminal
Description: GPU-accelerated terminal emulator
 Litty renders terminal sessions with Vulkan and supports tabs, splits,
 search, shell integration, and an applications-menu launcher.
CONTROL
chmod 755 "$stage" "$stage/DEBIAN"
chmod 644 "$stage/DEBIAN/control"
mkdir -p "$output"
dpkg-deb --root-owner-group -Zxz --build "$stage" "$output/litty_${version}_amd64.deb"
