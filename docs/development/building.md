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
5. Bake the release public key (`resources/minisign.pub`, or
   `LITTY_MINISIGN_PUBKEY` if set) into the binary so it can self-update.
6. Write `dist/SHA256SUMS.txt` for the macOS assets and, when `minisign` and
   a secret key (`~/.minisign/minisign.key`, or `MINISIGN_SECRET_KEY`) are
   available, sign it as `dist/SHA256SUMS.txt.minisig`.

---

Back to the [README](../../README.md).
