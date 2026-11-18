# v0.14.0 Release Validation

Status: release preparation (2026-11-18); not yet published.
Source: `0591708` plus v0.14.0 version metadata and documentation only.
No application source or dependency versions changed during preparation.
Release commit: to be filled after the last amend.

## Verification

### macOS arm64

The maintainer reported on 2026-11-18 that the macOS release checks pass. The
generated `scripts/release-test-report.md`, test counts and measured figures
were not supplied, so they are not archived here.

### Linux x86_64 (Ubuntu host, 2026-11-18, at `0591708`)

| Command | Result |
| --- | --- |
| `cargo check` | PASS |
| `cargo clippy --all-targets -- -D warnings` | PASS, zero warnings |
| `cargo test` outside sandbox | PASS: 866 unit + 1 + 14 shell tests, 17 ignored |
| `git diff --check` | PASS |
| `bash scripts/linux/test.sh` (Podman 5.7.0, Debian 13 image) | PASS, exit 0 |

The container run repeated check, Clippy and the full suite (866 + 1 + 14,
17 ignored) under Xvfb/D-Bus, then passed the X11 smoke (wheel, non-Latin-1
keysym, window close, Caps/Num Lock Kitty modifiers), the Weston smoke (open
with app id `litty`, configured resize, frame callbacks, close) and the sway
input smoke (shifted text, Ctrl+W, client key repeat, cursor shape, 150%
scale, title band, clean close).

CI on `main` at `0591708` passed.

## Known limitations

- Idle CPU and idle RAM were not re-measured for v0.14.0; the README badges keep
  the v0.13.0 figures (0.0%, 20 MB). Release binary sizes were not measured
  locally; v0.13.0 was 2,338,368 bytes (macOS) and 4,732,408 bytes (Linux).
- The macOS pre-release suite report is not archived (see above).
- The 17 ignored tests were not enabled; live AI providers and `xdg-open`
  dispatch need explicit runs.
- Linux packages (`.deb`, `.rpm`) are built and install/remove-tested by the
  release CI only; desktop launcher acceptance on them was not run.
- Open Wayland desktop acceptance carries to [v0.15.0](../plans/0.15.0.md).
- Host-only intermittent heap corruption in `cargo test` with `ibus-x11` input
  methods is still unexplained; it did not occur in these runs and has not
  been reported outside tests.
