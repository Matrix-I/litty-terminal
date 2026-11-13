# Litty Terminal ⚡

[![Build Status](https://img.shields.io/badge/build-passing-brightgreen.svg)]()
[![Version](https://img.shields.io/badge/version-0.12.0-blue.svg)]()
[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Linux-blue.svg)]()
[![GPU Backend](https://img.shields.io/badge/GPU-Metal%20%7C%20Vulkan-orange.svg)]()
[![Idle CPU](https://img.shields.io/badge/idle%20CPU-0.0%25-green.svg)]()
[![Idle RAM](https://img.shields.io/badge/idle%20RAM-63MB-success.svg)]()
[![Binary Size](https://img.shields.io/badge/binary-%3C%205MB-informational.svg)]()
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

**Litty** is an ultra-fast, lightweight, GPU-accelerated terminal emulator written in Rust. It delivers ~4 ms keystroke-to-frame latency, silky-smooth 120Hz ProMotion rendering, and an exceptionally small memory footprint without sacrificing modern developer ergonomics.

---

## ⚡ Highlights

- **Native GPU rendering** on Metal (macOS) and Vulkan (Linux): the whole grid in a single instanced draw call.
- **12-byte cells in a zero-allocation ring buffer**, so heavy output never thrashes the heap.
- **0.0% idle CPU** and 61 MB idle footprint with one window, 28 MB with all windows closed (`ControlFlow::Wait`, event-driven; see [Performance](docs/development/performance.md) for how each number is measured).
- **Ghostty-compatible configuration** with live reload.
- **Tabs, windows and splits** that inherit the working directory. Drag a tab title left or right to reorder tabs within a window, or drag it away from the tab bar to pull it out into a new window that follows the pointer, and move it over another window's tab bar to merge it there, as in Chrome.
- **Seamless title bar on Linux**: the window has no native frame; its title bar is drawn in the terminal's background color with minimize / maximize / close buttons, drag-to-move, double-click to maximize and resizable edges.
- **Built-in history search, regex find, click-to-position cursor and hyperlink detection.** Cmd/Ctrl+Click opens web links, folders and common text, source, document and media files; other files (scripts, apps, installers, `.terminal`, `.webloc`, files without an extension) are only revealed in their folder, and OSC 8 `file://` links from other hosts are ignored.
- **Automatic shell integration** (zsh, bash, fish, nushell, elvish) and the **Kitty keyboard / graphics protocols**, Sixel and full Unicode support.
- **`yo` AI shell assistant** with multiple providers, including local Ollama.
- **Safer paste and updates**: clipboard LF/CRLF line endings become terminal Enter bytes, while escape filtering remains active. Updates require a signed manifest bound to the exact release tag; macOS app updates stage and sign the complete bundle before an atomic swap. See [Updates](docs/guide/updates-and-cli.md).


See [Features](docs/guide/features.md) for the full list. All features have been tested
on Linux and run stably, as [confirmed by the user on 2026-11-04](docs/validation/linux-status-2026-11-04.md).

---

## 🚀 Quick Start

Download the [latest release](https://github.com/Matrix-I/litty-terminal/releases/latest), then follow the [installation guide](docs/guide/installation.md). On Debian/Ubuntu, install the `.deb` with `sudo apt install ./litty_0.8.0_amd64.deb` to add Litty to your applications menu with its icon. Fedora users can install `litty-0.8.0-1.x86_64.rpm` with `sudo dnf install ./litty-0.8.0-1.x86_64.rpm`. To build from source:

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
| [Building & Packaging](docs/development/building.md) | Build from source, macOS app bundle and DMG, Linux .deb/.rpm |
| [Release process](docs/development/release.md) | Maintainer release checklist |
| [Validation reports](docs/validation/) | Test and benchmark reports (AI live validation, Linux container run, scroll fix) |
| [Plans](docs/plans/) | Per-release plans and historical records (see also `ROADMAP.md`) |

---

## 📄 License

This project is licensed under the [MIT License](LICENSE).
