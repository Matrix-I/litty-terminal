# Native Linux container checks

Start Docker or Podman (allow at least 4 GiB for its Linux VM), then run from the
repository root:

```sh
bash scripts/linux/test.sh
```

The runner selects an active runtime, copies tracked source plus these scripts to
an isolated build context, and runs native Linux `cargo check`, Clippy with zero
warnings, the full Rust suite under Xvfb/D-Bus, shell PTY smoke checks, and an X11
window smoke test, plus a native Wayland window smoke test under headless Weston
(`wayland_smoke.py`: open, compositor-driven resize, frame callbacks, close), and a Wayland input smoke test under headless sway
(`wayland_sway_smoke.py`: typing through the keymap, Ctrl, client key repeat, pointer and cursor-shape binding, fractional scale). Rust is pinned to 1.99.1; the image uses Debian 13 and Mesa
software Vulkan. Named `litty-linux-target` and `litty-linux-registry` volumes
cache Linux build artifacts and public crates. Host credentials, user dotfiles,
`.git`, and the host `target` directory are not mounted into the container.

If the network uses a TLS proxy, supply a **public, already trusted** CA bundle:

```sh
LITTY_TEST_CA_BUNDLE=/path/to/trusted-ca-bundle.pem bash scripts/linux/test.sh
```

The optional build secret installs those public roots in the test image. TLS
verification stays enabled. Do not put private keys or credentials in that file.

The X11 probe uses Xvfb, Openbox and synthetic input to verify mouse wheel and
keysym input, window close, and Kitty lock modifier encoding. It does not prove physical keyboard behavior, Wayland
compositor integration, real browser/file-manager dispatch, or hardware GPU
latency/idle RAM limits. Keep those manual roadmap checks separate.

## Integrated Wayland validation

- [x] Merged-main X11, Weston and sway smoke checks, plus live GNOME IME,
  clipboard/drop and activation/occlusion checks have been run.
- [x] An isolated KWin 6.3.6 virtual compositor passed window-state,
  minimize/workspace recovery and startup-scale checks. Its temporary harness
  is separate from `test.sh`; the standard container script does not run KWin.
- [ ] Full KDE/sway desktop IME, clipboard payload/drop, physical input and
  mixed-DPI display checks remain open.

See the [integrated report](../../docs/validation/wayland-integrated-2026-11-14.md)
for the exact tested revision, environment failures/retries and acceptance scope.
