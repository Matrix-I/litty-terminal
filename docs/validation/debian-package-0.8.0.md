# v0.8.0 Debian package verification

Date: 2026-11-04. Packaging-only addition to the existing v0.8.0 release; the
Linux executable is copied unchanged from the published release asset.

## Environment and evidence

- macOS ARM host, Podman Debian 12 (`bookworm-slim`) amd64 container under
  emulation. Package: `litty_0.8.0_amd64.deb`.
- Verified the downloaded upstream `SHA256SUMS.txt` with
  `minisign -Vm SHA256SUMS.txt -p resources/minisign.pub`; compared the upstream
  manifest and executable with local release artifacts using `cmp`: passed.
- `bash -n scripts/linux/package-deb.sh scripts/linux/test-package-deb.sh`: passed.
- `shellcheck scripts/linux/package-deb.sh scripts/linux/test-package-deb.sh`:
  passed with no diagnostics.
- In Debian, `bash scripts/linux/test-package-deb.sh
  dist/v0.8.0/litty-linux-x86_64`: passed. Checks actual `.deb` metadata, exact
  executable/icon bytes, launcher fields and `desktop-file-validate`, file modes,
  root ownership, restrictive caller umask, and invalid inputs.
- Mutation check: removed `umask 022` in an isolated script copy and ran the same
  tests. They failed as expected; restoring the real script passes. Independent
  review discovered this permissions issue and the implementation now fixes it.
- `apt-get install -y --no-install-recommends
  /workspace/dist/v0.8.0/litty_0.8.0_amd64.deb`: passed. APT resolved runtime
  dependencies and configured `litty (0.8.0)` and the desktop-menu trigger.
- `apt-get remove -y litty` followed by absence checks for `/usr/bin/litty`,
  `/usr/share/applications/litty.desktop`, and
  `/usr/share/icons/hicolor/512x512/apps/litty.png`: passed.
- `readelf -d` confirms direct ELF dependencies are `libm.so.6` and `libc.so.6`;
  the package additionally declares the dynamically loaded desktop/Vulkan libraries.

## GUI validation limit

Attempted the installed launcher as a non-root user through `gio launch
/usr/share/applications/litty.desktop`, under Xvfb/Openbox and D-Bus, with updates
and shell integration disabled in an isolated home. The executable started, but
Mesa's software Vulkan driver aborted before a visible window appeared:

```text
LLVM ERROR: 64-bit code requested on a subtarget that doesn't support it!
AssertionError: No installed-launcher window appeared
```

The same attempt with Mesa's documented `GALLIUM_OVERRIDE_CPU_CAPS=sse2` and
`LP_NATIVE_VECTOR_WIDTH=128` did not resolve the error. Real amd64 Debian desktop,
Wayland, hardware GPU rendering, and dock pinning remain unverified. No application
code was changed to work around an emulation/driver limitation.

## Self-review

1. Requirements and correctness: package installs the unchanged executable,
   desktop entry and icon at standard system paths; metadata and invalid inputs
   have observable assertions.
2. Performance: packaging uses bounded files in a temporary directory; no runtime
   hot paths or SQL changed.
3. Security: no maintainer scripts, privilege changes, shell interpolation of
   terminal input, credentials, or secrets added; archive ownership is root/root.
4. Robustness: staging cleans up on exit, permissions do not depend on caller
   umask, architecture/version inputs are checked, and APT declares dependencies.
5. Syntax and naming: Bash syntax and ShellCheck pass; script names match the
   existing Linux tooling.
6. Modern APIs: uses `dpkg-deb --root-owner-group` and a standard desktop entry;
   no deprecated application API introduced.
7. Business clarity: amd64 and glibc 2.34 match the already published Linux asset;
   system package updates use APT and the launcher explicitly selects `/usr/bin`.
8. Test coverage: packaging suite passes and the permission test demonstrably
   fails without its fix; GUI end-to-end validation is incomplete as recorded above.
9. Design: packaging and packaging assertions are separate small scripts;
   existing icon and release executable are reused.

An independent review checked the diff and packaging files; its only blocking
finding was the corrected umask issue. Rust tests/build/Clippy and application
performance benchmarks were not rerun: no `src/`, Cargo files, or executable bytes
changed. The affected module is the packaging tooling, covered by the commands
above. Existing release performance values remain unchanged.
