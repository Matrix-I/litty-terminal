# Configuration

Litty reads its configuration from:
```text
~/.litty/config
```

- Press <kbd>Cmd</kbd> + <kbd>,</kbd> (macOS) or <kbd>Ctrl</kbd> + <kbd>,</kbd> (Linux) inside Litty to create a documented template and open it in your default `$EDITOR`.
- Press <kbd>Cmd</kbd> + <kbd>Shift</kbd> + <kbd>,</kbd> / <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>,</kbd> to live-reload.
- Configuration is also **automatically reloaded on window focus** whenever the file is modified on disk.
- Litty maintains full syntax compatibility with Ghostty configuration keys and loads custom themes from `~/.litty/themes` or `~/.config/ghostty/themes`.

## Configuration Reference (ConfigMap)

| Category | Option Key | Type / Accepted Values | Default | Description |
| :--- | :--- | :--- | :--- | :--- |
| **Window & Layout** | `window-width` | Integer $\ge 10$ | `None` (auto) | Initial terminal width in grid columns (must be set with `window-height`) |
| | `window-height` | Integer $\ge 4$ | `None` (auto) | Initial terminal height in grid rows (must be set with `window-width`) |
| | `background-opacity` | Float `0.0` .. `1.0` | `1.0` | Window transparency (0 = transparent, 1 = opaque) |
| | `background-blur` | `true`, `false`, or uint `0`..`255` | `0` (off) | macOS vibrancy blur radius behind transparent windows (`true` = 20 pt) |
| | `window-colorspace` | `srgb`, `display-p3` | `srgb` | Surface rendering color space (macOS) |
| | `window-padding-x` | Float (points) or `left,right` | `4.0` | Horizontal padding between window edge and terminal grid |
| | `window-padding-y` | Float (points) or `top,bottom` | `4.0` | Vertical padding between window edge and terminal grid |
| | `window-padding-balance` | `true`, `false` | `false` | Spread leftover window margin pixels evenly around the grid |
| | `macos-native-tabs` | `true`, `false` | `false` | Use macOS native NSWindow tab bar instead of built-in GPU tab bar |
| | `title` | String | Shell CWD / Title | Force a fixed window title instead of the running program's |
| **Fonts & Typography** | `font-family` | String (name or file path) | System monospace | Monospace font family. Repeatable to define fallback chain in priority order |
| | `font-size` | Float `6.0` .. `72.0` | `14.0` | Base font size in points (<kbd>Cmd+0</kbd> resets to this size) |
| | `adjust-cell-width` | Percentage (`10%`) or px (`-1`, `2`) | `0` | Adjust cell width metric |
| | `adjust-cell-height` | Percentage (`10%`) or px (`-1`, `2`) | `0` | Adjust cell height / line spacing |
| | `font-thicken` | `true`, `false` | `false` | Enable synthetic bold/thickening for thin fonts |
| | `font-feature` | Comma-separated or repeatable | None | OpenType font features (e.g. `-calt`, `ss01`, `liga=0`) |
| | `grapheme-width-method` | `unicode`, `legacy`, `wcswidth` | `unicode` | `unicode` enables DEC mode 2027 (emoji ZWJ clustering); `legacy` is 1 cell/codepoint |
| **Colors & Themes** | `background` | `#rrggbb` or named color | `#121316` | Default terminal background color |
| | `foreground` | `#rrggbb` or named color | `#e6e6e6` | Default terminal foreground color |
| | `theme` | Theme name or `light:X,dark:Y` | `None` | Color theme (861 bundled; dynamic light/dark OS switching supported) |
| | `palette` | `N=color` (0..15) | Default 16-color | Override specific ANSI palette index (`palette = 1=#ff0000`) |
| | `cursor-color` | `#rrggbb` | `foreground` | Initial cursor color (OSC 12 dynamically overrides per terminal) |
| | `selection-background` | `#rrggbb` | Default blue | Background fill for selected text |
| | `selection-foreground` | `#rrggbb` | Default light | Text color for selected text |
| | `minimum-contrast` | Float `1.0` .. `21.0` | `1.0` (off) | Enforce WCAG minimum contrast ratio between text & background |
| **Cursor & Mouse** | `cursor-style` | `block`, `bar`, `underline`, `block_hollow` | `block` | Initial cursor shape |
| | `cursor-style-blink` | `true`, `false`, or empty | Unset (app-set) | Cursor blinking state (`true`/`false` force overrides) |
| | `mouse-hide-while-typing` | `true`, `false` | `false` | Automatically hide cursor pointer when keystrokes occur |
| | `mouse-scroll-multiplier` | Float or `precision:P,discrete:D` | `precision:1,discrete:3` | Scroll speed factor for trackpads (precision) and wheels (discrete) |
| | `copy-on-select` | `true`, `false`, `clipboard` | `true` | Automatically copy text selection to system clipboard on mouse release |
| | `clipboard-write` | `true`, `allow`, `false`, `deny` | `true` | Allow programs to write to system clipboard via OSC 52 |
| **Shell Integration** | `shell-integration` | `detect`, `zsh`, `bash`, `fish`, `nushell`, `elvish`, `none` | `detect` | Automatic shell prompt mark and CWD reporting injection |
| | `shell-integration-features` | Comma-separated: `cursor`, `sudo`, `title` (or `no-*`) | `cursor,title` | Injected shell features (`cursor` sets bar prompt cursor; `title` reports CWD) |
| **Splits & Scrollback** | `scrollback-limit` | Integer (bytes) | `10000000` | Scrollback memory per terminal in bytes, as in Ghostty (about 10,400 lines at 80 columns) |
| | `unfocused-split-opacity` | Float `0.15` .. `1.0` | `0.7` | Opacity/dim factor applied to inactive split panes |
| | `unfocused-split-fill` | `#rrggbb` | Background | Color inactive splits fade toward |
| | `confirm-close-surface` | `true` (`running`), `false` (`never`), `always` | `running` | Ask confirmation before closing a pane with running child process |
| **Bell & Notifications** | `bell-features` | Comma-separated: `system`, `attention`, `title`, `audio`, `border` | `attention,title` | Actions triggered on terminal `BEL` (`\a`) |
| | `bell-audio-path` | Path to audio file | System Glass | Custom audio sound file for bell alerts |
| | `bell-audio-volume` | Float `0.0` .. `1.0` | `0.5` | Bell audio playback volume |
| **Shaders & Effects** | `custom-shader` | Path to GLSL file (repeatable) | None | Shadertoy GLSL post-processing shader pass (chains in file order). `mainImage` gets `iResolution`, `iTime`, `iTimeDelta`, `iFrame`, `iMouse`, `iDate` and `iChannel0` (the previous pass). A shader that does not compile is reported once on stderr with its path (on Linux also its line in your file) and skipped |
| | `custom-shader-animation` | `true` (focused), `false` (off), `always` | `true` | Animation frame loop policy for custom GLSL shaders |
| **Keybindings & Actions** | `keybind` | `trigger=action`, `trigger=text:<val>`, `unbind`, `clear` | Defaults | Custom shortcut mapping (Ghostty syntax) |
| **Auto Update** | `auto-update` | `check`, `download`, `off` (`false`) | `check` | GitHub self-update checking and downloading policy |
| | `auto-update-channel` | `stable`, `tip` | `stable` | Update release channel (`tip` includes prereleases) |

## Example `~/.litty/config`

```ini
# Font Configuration
font-family = "SF Mono"
font-family = "MesloLGS NF"
font-family = "Symbols Nerd Font Mono"
font-size = 14.0

# Window Appearance
background-opacity = 0.92
background-blur = 20
window-colorspace = "display-p3"
# window-width = 120
# window-height = 36

# Colors & Palette
background = #121316
foreground = #e6e6e6
# theme = "Catppuccin Mocha"     # any name from `litty +list-themes`
# theme = light:Horizon Bright,dark:Dracula

# Cursor
cursor-style = "block"          # "block", "bar", "underline", "block_hollow"
cursor-style-blink = true

# Mouse & Scrolling
mouse-hide-while-typing = true
mouse-scroll-multiplier = precision:1.0,discrete:3.0

# Shell Integration
shell-integration = "detect"     # "detect", "zsh", "bash", "fish", "nushell", "elvish", "none"
shell-integration-features = "cursor,title"

# Padding & Layout
window-padding-x = 8
window-padding-y = 6
window-padding-balance = true

# Colors & Behavior
# cursor-color = #f38ba8
# selection-background = #3d59a1
# selection-foreground = #ffffff
# copy-on-select = true
# title = Work
# bell-features = border,audio,no-attention
# bell-audio-path = /System/Library/Sounds/Glass.aiff
# font-feature = -calt          # ligatures are on with fonts that have them; also ss01, liga=0
# adjust-cell-height = 10%       # or pixels: 2 / -1; also adjust-cell-width
# font-thicken = true
# grapheme-width-method = unicode # unicode (DEC mode 2027) or legacy
# custom-shader = ~/.litty/shaders/crt.glsl   # Ghostty/Shadertoy GLSL; repeat to chain
# macos-native-tabs = false      # Cmd+T opens native macOS window tabs
# confirm-close-surface = running # running (default), never, always
# scrollback-limit = 10000000    # bytes, as in Ghostty (default)
# unfocused-split-opacity = 0.7  # fade inactive splits (0.15 - 1.0)
# unfocused-split-fill = #000000
# minimum-contrast = 1.1         # force readable text (1 = off, 21 = black/white)

# Auto Update
auto-update = "check"           # "check" (default), "download", or "off"
auto-update-channel = "stable"  # "stable" (default) or "tip"

# Key bindings (Ghostty syntax; the last match wins, `keybind = clear` drops them all)
keybind = cmd+t=new_split:right
keybind = ctrl+shift+h=text:hello\n
keybind = cmd+d=unbind
```

## Environment

| Variable | Values | Effect |
|---|---|---|
| `LITTY_BACKEND` | `x11` (default), `wayland` | Linux window system. `wayland` opens native Wayland windows (falls back to X11 if the compositor cannot be reached); it is a preview: keyboard, mouse, fractional scaling and Litty's own title bar (moving and resizing the window) work, but input methods (IME, dead keys) and occlusion do not yet, see the [Wayland backend plan](../plans/wayland-backend.md). |

## See also

- [Keybinding actions](keybinding-actions.md)
- [AI shell assistance](ai-assistant.md)

---

Back to the [README](../../README.md).
