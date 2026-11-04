# v0.8.0 RPM package verification

Date: 2026-11-04. Packaging-only addition: `litty-0.8.0-1.x86_64.rpm` wraps the
original signed v0.8.0 Linux executable without rebuilding or changing it.

## Environment and commands

macOS ARM host, Podman Fedora 43 amd64 container under QEMU, RPM 6.0.2.
The VM has approximately 1 GiB RAM. Only packaging inputs were copied into the
container; host credentials and repository metadata were not mounted.

- The existing signed release manifest passed `minisign -Vm SHA256SUMS.txt
  -p resources/minisign.pub`; the Linux executable matches its SHA-256 entry.
- `bash -n scripts/linux/package-rpm.sh scripts/linux/test-package-rpm.sh`:
  passed.
- `shellcheck scripts/linux/package-rpm.sh scripts/linux/test-package-rpm.sh`:
  passed without diagnostics.
- `bash scripts/linux/test-package-rpm.sh dist/v0.8.0/litty-linux-x86_64`:
  passed. Builds a real RPM under umask 077, extracts it, verifies metadata,
  unchanged executable/icon bytes, desktop fields and validation, modes,
  root ownership, automatic ELF requirements and explicit Vulkan requirements,
  absence of custom installed scriptlets, and invalid/missing/wrong-architecture
  inputs. Final build has no warnings.
- Mutation test: added `AutoReqProv: no` only to an isolated spec copy. That RPM
  built successfully, but the same test suite failed because the automatic
  `libc.so.6()(64bit)` requirement was absent. The real spec passes.
- `dnf -y --setopt=install_weak_deps=False install
  /workspace/dist/v0.8.0/litty-0.8.0-1.x86_64.rpm`: passed. DNF installed Litty and
  resolved the missing `libwayland-cursor`, `libwayland-egl`, and
  `libxkbcommon-x11` dependencies.
- `dnf check`: passed. `dnf -y --setopt=clean_requirements_on_remove=False
  remove litty` followed by absence checks for `/usr/bin/litty`,
  `/usr/share/applications/litty.desktop`, and the packaged icon: passed.
- `rpm -V litty`, `cmp /usr/bin/litty
  /workspace/dist/v0.8.0/litty-linux-x86_64`, and `rpm -qf` on the installed
  launcher/icon: passed.

The initial tool installation hit a proxy certificate error. Only certificates
from the existing public proxy CA bundle were installed in the container, and
TLS verification remained enabled. A subsequent large tool-install transaction
exited 137 after the tools were installed; `dnf check` found no consistency
errors, and the Litty build and DNF installation subsequently succeeded.

The RPM has no OpenPGP signature; DNF reported skipping that check for the local
package. Integrity is provided by its entry in the Minisign-signed release
checksum manifest. No RPM key or trust configuration is installed on users' systems.

## GUI validation scope

A non-root user launched `/usr/share/applications/litty.desktop` through
`gio launch`, under Xvfb/Openbox/D-Bus and software Vulkan, with an isolated home.
The first probe timed out waiting for a visible window after 60 seconds.
A subsequent probe accepted Litty's initial or configured title and allowed
120 seconds for emulated startup. The window name `Litty RPM package smoke` was
observable through X11, but the retry also timed out with
`AssertionError: No installed-launcher window appeared`. Visibility and
interactive PTY execution were not confirmed. This packaging-specific GUI check is incomplete; it does not alter
separately recorded user-reported Linux feature validation.

## Self-review

1. Correctness: installs the unchanged executable, shared desktop entry and icon;
   metadata, invalid inputs, and wrong architecture have observable assertions.
2. Performance: temporary staging is bounded by the existing asset files; no
   runtime hot paths or SQL changed.
3. Security: no custom installed scriptlets, secrets, or privilege/auth changes;
   files are root-owned and checksum verification preserves artifact provenance.
4. Robustness: temporary files are cleaned on exit; version/architecture are
   validated, modes are explicit, and both ELF and runtime-loaded dependencies
   are declared.
5. Syntax and naming: Bash syntax, ShellCheck and RPM build pass; names follow
   the existing Linux packaging scripts.
6. Modern APIs: uses standard RPM spec/build/query facilities and a validated
   desktop entry; no deprecated application APIs introduced.
7. Business clarity: targets the same x86_64/glibc 2.34 baseline as v0.8.0;
   DNF manages system-package updates and manual launcher precedence is documented.
8. Tests: the packaging suite passes and demonstrably fails with automatic ELF
   detection disabled; GUI visibility/interaction remain unverified as above.
9. Design: a focused packaging script, declarative spec and separate test script
   reuse the existing executable, launcher and icon.

Independent review found no blocking implementation issues; its requested checks
for automatic ELF requirements, icon modes and directory modes were added.
Rust tests/build/Clippy and app performance benchmarks were not rerun because
no `src/`, Cargo files, or executable bytes changed. Existing release performance
values remain unchanged; only the RPM packaging tooling is newly exercised.
