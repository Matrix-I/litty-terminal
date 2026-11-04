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
header=$(od -An -tx1 -N20 "$binary" | tr -d ' \n')
if [[ ! "$header" =~ ^7f454c46020101[0-9a-f]{18}0[23]003e00$ ]]; then
    echo 'BINARY must be a little-endian Linux x86_64 ELF executable.' >&2
    exit 1
fi
command -v rpmbuild >/dev/null
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage/SOURCES" "$stage/SPECS"
install -m 755 "$binary" "$stage/SOURCES/litty"
install -m 644 "$root/resources/linux/litty.desktop" "$stage/SOURCES/litty.desktop"
install -m 644 "$root/resources/icon.png" "$stage/SOURCES/icon.png"
cat > "$stage/SOURCES/README.rpm" <<'DOC'
Launch Litty Terminal from your desktop's applications menu or run /usr/bin/litty.
This package requires a working Vulkan driver and an X11 or Wayland desktop.
Install newer RPM releases with your package manager; the built-in updater cannot
replace root-owned /usr/bin/litty as an ordinary user. Do not run Litty as root
to update. A previous ~/.local/share/applications/litty.desktop overrides the
packaged launcher. Remove that old manual shortcut if it points elsewhere.
DOC
sed "s/@VERSION@/$version/" "$root/resources/linux/litty.spec" > "$stage/SPECS/litty.spec"
rpmbuild -bb --target x86_64 --define "_topdir $stage" "$stage/SPECS/litty.spec"
mkdir -p "$output"
install -m 644 "$stage/RPMS/x86_64/litty-$version-1.x86_64.rpm" "$output/litty-$version-1.x86_64.rpm"
