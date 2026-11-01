# Linux container verification — 2026-11-01

Source baseline: `52fce2f`, plus the working-tree Linux Clippy/Quick Terminal fixes and test
harness. This was a native **aarch64 Linux** run in Podman on Apple Silicon,
using Debian 13, Rust 1.99.1, Xvfb/D-Bus, and Mesa software Vulkan. It was not a
cross-compilation-only check.

| Check | Result |
| --- | --- |
| `cargo check --locked` | Passed |
| `cargo clippy --locked --all-targets -- -D warnings` | Passed |
| `cargo test --locked -- --test-threads=1` under Xvfb | **278 passed, 0 failed, 3 ignored** |
| zsh, Bash, fish PTY assistant smoke checks | Passed for all three shells |
| X11 Kitty Caps/Num Lock via raw XTEST, report-all Kitty mode | Passed |
| X11 Quick Terminal global hotkey, geometry, socket restore, shell reuse, autohide | Passed |
| macOS and x86_64 Linux cross-target compile after the fix | Passed |
| macOS Kitty file-medium security regression and Clippy | Passed |

The complete pipeline was repeated after both production fixes and exited 0,
including both X11 PASS checks. The three ignored tests are existing manual
performance benchmarks. The native
Linux suite has fewer tests than macOS because platform-specific macOS tests are
excluded by `cfg`.

Native Clippy found a Linux-only `useless_vec` warning in `is_temp_dir`: only the
macOS branch appended to the Vec. The fix uses an array with conditional macOS
entries, preserving path validation while removing the allocation.

The running-app X11 probe also exposed transient focus events during Quick
Terminal creation and global keyboard grabs. Quick Terminal now starts hidden
with its final decorations/window level, and autohide waits for a single 50 ms
deadline which focus regain or a hotkey toggle cancels. This prevents hiding and
immediately reopening on the second hotkey. It adds no periodic polling. The
probe confirms normal focus loss still hides the panel and reopening reuses the
same shell.

To repeat with an active Docker/Podman runtime (4 GiB VM RAM recommended):

```sh
bash scripts/linux/test.sh
```

See [the test runner instructions](../scripts/linux/README.md) for optional
public CA-bundle configuration on networks using a TLS proxy. TLS verification
remains enabled, and host credentials/user configuration are not mounted.

Physical keyboard/desktop verification, Wayland compositor integration, actual
browser/file-manager opening, live LLM providers, Nushell, and hardware GPU
performance/RAM limits are outside the native unit-test result above.

After validation, the three test containers and their intermediate test images
were removed. The Podman VM was stopped and its RAM restored to 1 GiB.
