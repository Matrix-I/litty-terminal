# Installation

## macOS (Pre-built DMG / App)

1. Download **`Litty.dmg`** (or `Litty-macos.zip`) from the [Latest Release](https://github.com/Matrix-I/litty-terminal/releases/latest).
2. Open `Litty.dmg` and drag `Litty.app` into your `/Applications` folder.
3. **macOS Gatekeeper Notice**: Because Litty is not yet notarized with an Apple Developer Certificate, macOS will mark downloaded apps with a quarantine attribute. Run the following command in Terminal to allow it to open:

```bash
xattr -dr com.apple.quarantine /Applications/Litty.app
```

Now you can launch Litty from Spotlight, Launchpad, or the Applications folder!

## Linux (Pre-built Binary)

1. Download **`litty-linux`** (or `litty-linux-x86_64.tar.gz`) from the [Latest Release](https://github.com/Matrix-I/litty-terminal/releases/latest).
2. Make it executable and place it in your `$PATH`:

```bash
chmod +x litty-linux
sudo mv litty-linux /usr/local/bin/litty
```

---

Back to the [README](../README.md).
