# Litty Terminal ⚡

[![Build Status](https://img.shields.io/badge/build-passing-brightgreen.svg)]()
[![Version](https://img.shields.io/badge/version-0.7.1-blue.svg)]()
[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Linux-blue.svg)]()
[![GPU Backend](https://img.shields.io/badge/GPU-Metal%20%7C%20Vulkan-orange.svg)]()
[![Idle CPU](https://img.shields.io/badge/idle%20CPU-0.0%25-green.svg)]()
[![Idle RAM](https://img.shields.io/badge/idle%20RAM-~36MB-success.svg)]()
[![Binary Size](https://img.shields.io/badge/binary-5.7MB-informational.svg)]()
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

**Litty** is an ultra-fast, lightweight, GPU-accelerated terminal emulator written in Rust. It delivers sub-millisecond input response, silky-smooth 120Hz ProMotion rendering, and an exceptionally small memory footprint without sacrificing modern developer ergonomics.

---

## ⚡ Highlights

- **Native GPU rendering** on Metal (macOS) and Vulkan (Linux): the whole grid in a single instanced draw call.
- **12-byte cells in a zero-allocation ring buffer**, so heavy output never thrashes the heap.
- **0.0% idle CPU** and ~36 MB idle RAM (`ControlFlow::Wait`, event-driven).
- **Ghostty-compatible configuration** with live reload.
- **Tabs, windows and splits** that inherit the working directory.
- **Built-in history search, regex find, click-to-position cursor and hyperlink detection.**
- **Automatic shell integration** (zsh, bash, fish, nushell, elvish) and the **Kitty keyboard / graphics protocols**, Sixel and full Unicode support.
- **`yo` AI shell assistant** with multiple providers, including local Ollama.

See [Features](docs/guide/features.md) for the full list.

---

## 🚀 Quick Start

Download the [latest release](https://github.com/Matrix-I/litty-terminal/releases/latest), then follow the [installation guide](docs/guide/installation.md). To build from source:

```bash
git clone https://github.com/Matrix-I/litty-terminal.git
cd litty-terminal
cargo build --release
```

Press <kbd>Cmd</kbd> + <kbd>,</kbd> (macOS) or <kbd>Ctrl</kbd> + <kbd>,</kbd> (Linux) inside Litty to create and open `~/.litty/config`.

---

## 📚 Documentation

| Topic | Description |
| :--- | :--- |
| [Features](docs/guide/features.md) | Everything Litty can do: rendering, input protocols, Unicode, images, themes |
| [Performance & Memory Footprint](docs/development/performance.md) | Comparison with other terminals and engine microbenchmarks |
| [Architecture](docs/development/architecture.md) | PTY reader / render thread design |
| [Default Shortcuts](docs/guide/shortcuts.md) | macOS and Linux shortcut matrix |
| [Configuration](docs/guide/configuration.md) | Config file location, option reference, example config |
| [Keybinding Actions](docs/guide/keybinding-actions.md) | Actions available to `keybind = trigger=action` |
| [AI Shell Assistance (`yo`)](docs/guide/ai-assistant.md) | Providers, keys and workflow |
| [Auto-Update & CLI Commands](docs/guide/updates-and-cli.md) | `litty +update`, channels, background checks |
| [Installation](docs/guide/installation.md) | Pre-built macOS and Linux binaries |
| [Building & Packaging](docs/development/building.md) | Build from source, app bundle and DMG |
| [Release process](docs/development/release.md) | Maintainer release checklist |
| [Validation reports](docs/validation/) | Test and benchmark reports (AI live validation, Linux container run, scroll fix) |
| [Plans](docs/plans/) | Per-release plans and historical records (see also `ROADMAP.md`) |

---

## 📄 License

This project is licensed under the [MIT License](LICENSE).
