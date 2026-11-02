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

---

Back to the [README](../README.md).
