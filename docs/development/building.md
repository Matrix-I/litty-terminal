# Building & Packaging

## Prerequisites

- [Rust & Cargo](https://rustup.rs/) (edition 2021, Rust 1.80+)
- **macOS:** Xcode Command Line Tools (`xcode-select --install`)
- **Linux:** Vulkan drivers, `libxkbcommon`, `fontconfig`

## Build from Source

```bash
# Clone the repository
git clone https://github.com/Matrix-I/litty-terminal.git
cd litty-terminal

# Run tests
cargo test

# Build optimized release binary
cargo build --release
```

## Fast Inner-Loop Builds & Compile-Time Optimization

To accelerate iterative builds and debug cycles:

### 1. Fast Linker (`mold`)
On Linux, enable the multi-threaded `mold` linker for instant incremental links:
```bash
# Install clang and mold
sudo apt-get install -y clang mold   # Debian / Ubuntu
sudo dnf install -y clang mold       # Fedora

# Enable config template
cp .cargo/config.toml.example .cargo/config.toml
```

### 2. Compilation Caching (`sccache`)
To share build artifacts across git branch switches and clean builds:
```bash
# Install sccache
cargo install sccache

# Enable rustc wrapper
export RUSTC_WRAPPER=sccache
```

### 3. Profiling Compile Times (`cargo build --timings`)
To inspect pipeline bottlenecks and crate compilation graphs:
```bash
cargo build --timings
# Report generated at target/cargo-timings/cargo-timing.html
```

## macOS Application Bundle & DMG

Run the automated bundling script:

```bash
./bundle_macos.sh
```

This will:
1. Build Litty with aggressive LTO and size stripping.
2. Generate `Litty.app` signed with ad-hoc certificates.
3. Install `Litty.app` to `~/Applications/Litty.app`.
4. Create an installable drag-and-drop disk image: `dist/Litty.dmg`.
5. Bake the release public key (`resources/minisign.pub`, or
   `LITTY_MINISIGN_PUBKEY` if set) into the binary so it can self-update.
6. Write `dist/SHA256SUMS.txt` for the macOS assets and, when `minisign` and
   a secret key (`~/.minisign/minisign.key`, or `MINISIGN_SECRET_KEY`) are
   available, sign it as `dist/SHA256SUMS.txt.minisig`.

## Linux Debian Package

On Debian, package an existing Linux x86_64 release executable (requires
`dpkg-deb` from `dpkg`; tests also use `binutils` and `desktop-file-utils`):

```bash
bash scripts/linux/package-deb.sh dist/v0.8.0/litty-linux-x86_64 0.8.0 dist/v0.8.0
bash scripts/linux/test-package-deb.sh dist/v0.8.0/litty-linux-x86_64
```

The resulting `litty_0.8.0_amd64.deb` includes `/usr/bin/litty`, a desktop launcher,
and the existing Litty icon. The executable is copied unchanged; build it with
the release key and glibc 2.34 baseline as described in the [release process](release.md).

## Linux RPM Package

On Fedora, install the packaging tools and wrap the existing x86_64 release
executable without recompiling it:

```bash
sudo dnf install rpm-build binutils cpio desktop-file-utils
bash scripts/linux/package-rpm.sh dist/v0.8.0/litty-linux-x86_64 0.8.0 dist/v0.8.0
bash scripts/linux/test-package-rpm.sh dist/v0.8.0/litty-linux-x86_64
```

The output is `litty-0.8.0-1.x86_64.rpm`. The spec preserves executable bytes,
keeps automatic ELF dependency detection, and declares libraries loaded at runtime.
It installs the shared Linux desktop entry and icon without custom scriptlets.

---

Back to the [README](../../README.md).
