#!/usr/bin/env bash
# Run inside the Linux test image, with software Vulkan and an isolated X server.
set -euo pipefail
export XDG_RUNTIME_DIR
XDG_RUNTIME_DIR=$(mktemp -d)
trap 'rm -rf "$XDG_RUNTIME_DIR"' EXIT
chmod 700 "$XDG_RUNTIME_DIR"
export WGPU_BACKEND=vulkan
export SHELL=/bin/bash
unset WAYLAND_DISPLAY
uname -sm
rustc --version
cargo check --locked
cargo clippy --locked --all-targets -- -D warnings
xvfb-run -a -s '-screen 0 1280x800x24' dbus-run-session -- \
    cargo test --locked -- --test-threads=1
cargo test --locked linux_xdg_open_dispatch_smoke -- --ignored --nocapture
cargo build --locked
xvfb-run -a -s '-screen 0 1280x800x24' dbus-run-session -- \
    python3 scripts/linux/x11_smoke.py target/debug/litty
