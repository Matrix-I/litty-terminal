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
(`wayland_smoke.py`: open, compositor-driven resize, frame callbacks, close). Rust is pinned to 1.99.1; the image uses Debian 13 and Mesa
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
