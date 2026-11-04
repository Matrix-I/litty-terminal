#!/usr/bin/env bash
# Run on Fedora with rpm-build, binutils, cpio, and desktop-file-utils installed.
set -euo pipefail
export LC_ALL=C
root=$(cd "$(dirname "$0")/../.." && pwd)
if [[ $# != 1 ]]; then
    echo "Usage: $0 RELEASE_BINARY" >&2
    exit 1
fi
binary=$1
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
package="$work/output/litty-0.8.0-1.x86_64.rpm"
(umask 077; bash "$root/scripts/linux/package-rpm.sh" "$binary" 0.8.0 "$work/output")
[[ $(rpm -qp --qf '%{NAME} %{VERSION} %{RELEASE} %{ARCH}' "$package") == 'litty 0.8.0 1 x86_64' ]]
rpm -qp --requires "$package" > "$work/requires"
grep -Fxq 'libc.so.6()(64bit)' "$work/requires"
grep -Fxq 'glibc >= 2.34' "$work/requires"
grep -Fxq 'libvulkan.so.1()(64bit)' "$work/requires"
[[ -z $(rpm -qp --scripts "$package") ]]
mkdir "$work/extracted"
(cd "$work/extracted"; rpm2cpio "$package" | cpio -idm --quiet)
cmp "$binary" "$work/extracted/usr/bin/litty"
cmp "$root/resources/icon.png" "$work/extracted/usr/share/icons/hicolor/512x512/apps/litty.png"
desktop="$work/extracted/usr/share/applications/litty.desktop"
desktop-file-validate "$desktop"
grep -Fxq 'Exec=/usr/bin/litty' "$desktop"
grep -Fxq 'Terminal=false' "$desktop"
grep -Fxq 'Icon=litty' "$desktop"
[[ $(stat -c %a "$work/extracted/usr/bin/litty") == 755 ]]
[[ $(stat -c %a "$desktop") == 644 ]]
[[ $(stat -c %a "$work/extracted/usr/share/icons/hicolor/512x512/apps/litty.png") == 644 ]]
[[ $(stat -c %a "$work/extracted/usr/share/doc/litty/README.rpm") == 644 ]]
while IFS= read -r directory; do
    [[ $(stat -c %a "$directory") == 755 ]]
done < <(find "$work/extracted/usr" -type d)
rpm -qp --qf '[%{FILEUSERNAME} %{FILEGROUPNAME}\n]' "$package" | awk '$0 != "root root" { bad=1 } END { exit bad }'
readelf -h "$work/extracted/usr/bin/litty" | grep -q 'Advanced Micro Devices X86-64'
expect_failure() {
    if bash "$root/scripts/linux/package-rpm.sh" "$@" > "$work/error" 2>&1; then
        echo "Expected packaging to reject: $*" >&2
        exit 1
    fi
}
expect_failure
expect_failure "$binary" '0.8.0/invalid' "$work/output"
grep -q 'VERSION must' "$work/error"
expect_failure "$work/missing" 0.8.0 "$work/output"
grep -q 'BINARY must' "$work/error"
printf 'not an ELF executable\n' > "$work/invalid"
expect_failure "$work/invalid" 0.8.0 "$work/output"
grep -q 'ELF executable' "$work/error"
head -c 20 "$binary" > "$work/wrong-architecture"
printf '\267\000' | dd of="$work/wrong-architecture" bs=1 seek=18 conv=notrunc status=none
expect_failure "$work/wrong-architecture" 0.8.0 "$work/output"
grep -q 'ELF executable' "$work/error"
echo 'RPM packaging tests passed (payload, metadata, launcher, icon, modes, ownership, dependencies, invalid inputs).'
