# Installation

## macOS (Pre-built DMG / App)

1. Download **`Litty.dmg`** from the [Latest Release](https://github.com/Matrix-I/litty-terminal/releases/latest).
2. Open `Litty.dmg` and drag `Litty.app` into your `/Applications` folder.
3. **macOS Gatekeeper Notice**: Because Litty is not yet notarized with an Apple Developer Certificate, macOS will mark downloaded apps with a quarantine attribute. Run the following command in Terminal to allow it to open:

```bash
xattr -dr com.apple.quarantine /Applications/Litty.app
```

Now you can launch Litty from Spotlight, Launchpad, or the Applications folder!

## Debian / Ubuntu (.deb, amd64)

Download **`litty_0.8.0_amd64.deb`** from the [v0.8.0 release](https://github.com/Matrix-I/litty-terminal/releases/tag/v0.8.0), then install it from the download directory:

```bash
sudo apt install ./litty_0.8.0_amd64.deb
```

Open your applications menu and search for **Litty Terminal**. The package installs
`/usr/bin/litty`, the launcher, and the Litty icon. You can pin it to your dock or
taskbar using your desktop's favorites/pin option. Requires x86_64, glibc 2.34+
(Debian 12 or newer), and a working Vulkan driver.

If you previously created `~/.local/share/applications/litty.desktop` manually,
remove that old shortcut so it does not override the packaged launcher. A previous
`/usr/local/bin/litty` may also take precedence when running `litty` from a shell;
the packaged menu launcher always uses `/usr/bin/litty`.

Install future `.deb` updates with `sudo apt install ./<new-package>.deb`.
The built-in updater cannot replace the root-owned binary as an ordinary user;
do not run Litty as root to update. Uninstall with `sudo apt remove litty`.

## Fedora (.rpm, x86_64)

Download **`litty-0.8.0-1.x86_64.rpm`** from the [v0.8.0 release](https://github.com/Matrix-I/litty-terminal/releases/tag/v0.8.0), then install it from the download directory:

```bash
sudo dnf install ./litty-0.8.0-1.x86_64.rpm
```

Open the applications menu and search for **Litty Terminal**. The RPM includes
`/usr/bin/litty`, the same launcher and icon as the Debian package, and requires
a working Vulkan driver and glibc 2.34+. Use DNF to install future RPM updates;
the built-in updater cannot replace the root-owned executable as an ordinary
user. Uninstall with `sudo dnf remove litty`.

If you previously created `~/.local/share/applications/litty.desktop`, remove
that manual shortcut so it does not override the packaged launcher. A previous
`/usr/local/bin/litty` may take precedence in a shell; the packaged menu entry
explicitly launches `/usr/bin/litty`.

The RPM itself is not RPM-signed. Verify the download with the release's
`SHA256SUMS.txt` and its Minisign signature. Other RPM distributions are not
validated by the Fedora installation check.

## Other Linux distributions (Pre-built Binary)

1. Download **`litty-linux-x86_64`** from the [Latest Release](https://github.com/Matrix-I/litty-terminal/releases/latest).
2. Make it executable and place it in your `$PATH`:

```bash
chmod +x litty-linux-x86_64
sudo mv litty-linux-x86_64 /usr/local/bin/litty
```

---

Back to the [README](../../README.md).
