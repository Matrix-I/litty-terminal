#!/usr/bin/env bash
# Exercise bundle_linux.sh in a copy of the repository with a stub `cargo`, so
# the flow (build, key pinning, packaging, failure handling) is checked without
# a multi-minute release build. Run on Linux x86_64 with dpkg-deb installed.
set -euo pipefail
export LC_ALL=C
root=$(cd "$(dirname "$0")/../.." && pwd)
fake_elf=/usr/bin/true
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

copy=$work/repo
mkdir -p "$copy/scripts/linux" "$copy/resources" "$work/bin" "$work/home"
cp "$root/bundle_linux.sh" "$root/Cargo.toml" "$copy/"
cp "$root/scripts/linux/package-deb.sh" "$root/scripts/linux/package-rpm.sh" "$copy/scripts/linux/"
cp -R "$root/resources/linux" "$copy/resources/"
cp "$root/resources/icon.png" "$root/resources/minisign.pub" "$copy/resources/"
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$copy/Cargo.toml" | head -n 1)
key=$(tail -n 1 "$copy/resources/minisign.pub")

# Records how it was invoked, then "builds" by copying a real x86_64 ELF.
cat > "$work/bin/cargo" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" > "$STUB_LOG"
printf '%s\n' "${LITTY_MINISIGN_PUBKEY:-}" > "$STUB_KEY"
[ "${STUB_FAIL:-0}" = 1 ] && exit 101
mkdir -p target/release
cp "$FAKE_ELF" target/release/litty
STUB
chmod +x "$work/bin/cargo"

# HOME points at an empty directory so ~/.cargo/env cannot put the real cargo
# ahead of the stub.
run_bundle() {
    env -u CARGO_TARGET_DIR -u LITTY_MINISIGN_PUBKEY HOME="$work/home" \
        PATH="$work/bin:$PATH" STUB_LOG="$work/cargo-args" STUB_KEY="$work/cargo-key" \
        FAKE_ELF="$fake_elf" "$@" bash "$copy/bundle_linux.sh"
}

# Happy path.
run_bundle > "$work/out" 2>&1 || { cat "$work/out" >&2; exit 1; }
[[ $(cat "$work/cargo-args") == 'build --release' ]]
[[ $(cat "$work/cargo-key") == "$key" ]]
cmp "$fake_elf" "$copy/dist/litty-linux-x86_64"
[[ $(stat -c %a "$copy/dist/litty-linux-x86_64") == 755 ]]
deb=$copy/dist/litty_${version}_amd64.deb
[[ $(dpkg-deb -f "$deb" Version) == "$version" ]]
dpkg-deb --fsys-tarfile "$deb" | tar -xOf - ./usr/bin/litty | cmp - "$fake_elf"
rpm=$copy/dist/litty-$version-1.x86_64.rpm
if command -v rpmbuild >/dev/null; then
    test -s "$rpm"
else
    test ! -e "$rpm"
    grep -q 'rpmbuild not installed' "$work/out"
fi

# A key set by the caller wins over resources/minisign.pub.
run_bundle LITTY_MINISIGN_PUBKEY=RWQcallerkey > "$work/out" 2>&1 || { cat "$work/out" >&2; exit 1; }
[[ $(cat "$work/cargo-key") == RWQcallerkey ]]

# A failed build must stop before packaging anything.
rm -rf "$copy/dist" "$copy/target" "$work/cargo-args"
if run_bundle STUB_FAIL=1 > "$work/out" 2>&1; then
    echo 'Expected bundle_linux.sh to fail when cargo fails' >&2
    exit 1
fi
test -e "$work/cargo-args"
test ! -e "$copy/dist"

# Unexpected arguments are rejected before building.
rm -f "$work/cargo-args"
if env HOME="$work/home" PATH="$work/bin:$PATH" STUB_LOG="$work/cargo-args" \
    STUB_KEY="$work/cargo-key" FAKE_ELF="$fake_elf" \
    bash "$copy/bundle_linux.sh" --install > "$work/out" 2>&1; then
    echo 'Expected bundle_linux.sh to reject arguments' >&2
    exit 1
fi
grep -q 'Usage' "$work/out"
test ! -e "$work/cargo-args"

# An unreadable version is rejected before building.
sed -i 's/^version = ".*"$/version.workspace = true/' "$copy/Cargo.toml"
if run_bundle > "$work/out" 2>&1; then
    echo 'Expected bundle_linux.sh to reject a missing version' >&2
    exit 1
fi
grep -q 'MAJOR.MINOR.PATCH' "$work/out"
test ! -e "$work/cargo-args"

echo 'Linux bundle tests passed (build flags, pinned key, key override, binary copy, packages, build failure, arguments, version).'
