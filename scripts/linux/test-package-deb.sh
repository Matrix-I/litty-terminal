#!/usr/bin/env bash
# Run on Debian with dpkg-deb, binutils, and desktop-file-utils installed.
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
package="$work/output/litty_0.8.0_amd64.deb"
(umask 077; bash "$root/scripts/linux/package-deb.sh" "$binary" 0.8.0 "$work/output")
[[ $(dpkg-deb -f "$package" Package) == litty ]]
[[ $(dpkg-deb -f "$package" Version) == 0.8.0 ]]
[[ $(dpkg-deb -f "$package" Architecture) == amd64 ]]
dpkg-deb -f "$package" Depends | grep -Fq 'libc6 (>= 2.34)'
dpkg-deb -x "$package" "$work/extracted"
cmp "$binary" "$work/extracted/usr/bin/litty"
cmp "$root/resources/icon.png" "$work/extracted/usr/share/icons/hicolor/512x512/apps/litty.png"
desktop="$work/extracted/usr/share/applications/litty.desktop"
desktop-file-validate "$desktop"
grep -Fxq 'Exec=/usr/bin/litty' "$desktop"
grep -Fxq 'Terminal=false' "$desktop"
grep -Fxq 'Icon=litty' "$desktop"
[[ $(stat -c %a "$work/extracted/usr/bin/litty") == 755 ]]
[[ $(stat -c %a "$desktop") == 644 ]]
[[ $(stat -c %a "$work/extracted/usr/share/doc/litty/README.Debian") == 644 ]]
while IFS= read -r directory; do
    [[ $(stat -c %a "$directory") == 755 ]]
done < <(find "$work/extracted/usr" -type d)
dpkg-deb -c "$package" | awk '$2 != "root/root" { bad=1 } END { exit bad }'
readelf -h "$work/extracted/usr/bin/litty" | grep -q 'Advanced Micro Devices X86-64'
expect_failure() {
    if bash "$root/scripts/linux/package-deb.sh" "$@" > "$work/error" 2>&1; then
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
echo 'Debian packaging tests passed (payload, metadata, launcher, icon, modes, ownership, invalid inputs).'
