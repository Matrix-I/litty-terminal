# Auto-Update & CLI Commands

Litty includes a built-in, zero-dependency self-updater that checks for releases on GitHub, validates SHA-256 checksums from `SHA256SUMS.txt`, and updates `Litty.app` (with ad-hoc codesigning) or standalone binaries.

- **macOS Menu Bar**: Choose **Litty ▸ Check for Updates…** to check and install updates interactively with native Cocoa alerts.
- **CLI Commands**:
  - `litty +check-update` (or `--check-update`): Check if a new version is available without installing.
  - `litty +update` (or `--update`): Download and install the newest release directly in your terminal (use `--force` to reinstall current version).
- **Background checks** (at most once a day, at startup): `auto-update = check` notifies when a release is out; `auto-update = download` also downloads and verifies it, and the next **Check for Updates…** / `litty +update` installs that copy without downloading again. Nothing is installed without asking.
- **Channels**: `stable` follows the latest release; `tip` also considers prereleases and picks the highest version.
- **Architecture-safe**: the updater picks the asset for this machine's CPU (`litty-macos-<arch>`, `Litty-macos-<arch>.zip`, `litty-linux-<arch>`, with `<arch>` = `aarch64` or `x86_64`; the unsuffixed names of older releases count as arm64 on macOS and x86_64 on Linux) and refuses a binary whose Mach-O / ELF header is built for another CPU.
- **Version-bound signatures**: the signed `SHA256SUMS.txt` must contain exactly one `# litty-release: <tag>` line matching the requested GitHub tag, including prerelease suffixes. Older manifests without this marker are rejected; use a manual install for those releases.
- **Atomic macOS app updates**: the updater extracts and signs a complete bundle in a private directory beside the installed app, then swaps both directories atomically. Staging, signing or swap failures leave the installed bundle intact. A filesystem without atomic swap support returns an error.
- Only one update runs at a time (a lock in `~/.cache/litty/updates`, shared with `litty +update`), and every network request has connect and transfer timeouts.

---

Back to the [README](../../README.md).
