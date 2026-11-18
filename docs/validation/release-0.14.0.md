# v0.14.0 Release Validation

Status: published as [v0.14.0](https://github.com/Matrix-I/litty-terminal/releases/tag/v0.14.0)
on 2026-11-18 (14:58:03 UTC).
Source: `0591708` plus v0.14.0 version metadata and documentation only.
No application source or dependency versions changed during preparation.
Release commit: `2d915ff` (`chore(release): prepare v0.14.0`), pushed to `main`.

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
  locally; the CI binaries are listed below.
- The macOS pre-release suite report is not archived (see above).
- The 17 ignored tests were not enabled; live AI providers and `xdg-open`
  dispatch need explicit runs.
- Linux packages (`.deb`, `.rpm`) are built and install/remove-tested by the
  release CI only; desktop launcher acceptance on them was not run.
- Open Wayland desktop acceptance carries to [v0.15.0](../plans/0.15.0.md).
- Host-only intermittent heap corruption in `cargo test` with `ibus-x11` input
  methods is still unexplained; it did not occur in these runs and has not
  been reported outside tests.

## Final CI and published-release verification

[Release workflow 44328581878](https://github.com/Matrix-I/litty-terminal/actions/runs/44328581878)
completed successfully at tagged commit `2d915ff`: version check, macOS
aarch64, Linux x86_64, Debian and Fedora packages, signing and draft
creation. CI run 44328577037 on `main` at the same commit also passed.

Checked on Ubuntu x86_64 after downloading every asset of the draft:

- All eight release assets are present; all six SHA-256 hashes verify.
  The manifest contains exactly one `# litty-release: v0.14.0` marker.
- The minisign signature (and its trusted comment) verifies against
  `resources/minisign.pub` with minisign 0.12, after publication.
- CI binaries: macOS 2,338,384 bytes; Linux x86_64 4,818,464 bytes. Both
  contain version 0.14.0 and the pinned public key and are below 5,000,000
  bytes.
- App ZIP: both CFBundle versions equal 0.14.0. Its code signature was not
  checked (no macOS host in this check).
- The draft's generated notes were replaced with the prepared release notes
  before publication; the release is the repository's latest, not a
  prerelease.
- Older-build update flow passed on Linux: the v0.13.0 standalone binary,
  verified against the v0.13.0 manifest, in a scratch directory with an
  isolated HOME/XDG config and cache, ran `+check-update` (detects v0.14.0),
  `+update` (checksum verified, installed) and `+check-update` again
  (current v0.14.0, already latest). The installed file matches the
  published `litty-linux-x86_64` hash. A first `+update` attempt without
  `minisign` installed failed closed and left the binary unchanged. No
  installed user app was modified. The macOS updater and packaged-app
  replacement were not exercised.
