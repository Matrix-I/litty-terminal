# Keybinding Actions (`keybind = trigger=action`)

Custom shortcuts can be bound to any action below:

```ini
# Syntax: keybind = <modifiers>+<key>=<action>
# Example:
keybind = cmd+t=new_split:right
keybind = ctrl+shift+h=text:hello\n
keybind = cmd+d=unbind
```

| Action Name | Description |
| :--- | :--- |
| `new_tab` | Create a new tab in current working directory |
| `new_window` | Create a new window in current working directory |
| `close_surface` / `close_tab` | Close active pane (or tab if last pane) |
| `quit` | Quit terminal application |
| `toggle_minimize` / `minimize` | Minimize window |
| `new_split:right` / `new_split:down` | Split active pane vertically (right) or horizontally (down) |
| `goto_split:left\|right\|up\|down\|next\|previous` | Move focus spatially across split panes |
| `resize_split:left\|right\|up\|down` | Dynamically expand or shrink active split ratio |
| `equalize_splits` | Reset all split divider ratios evenly |
| `toggle_split_zoom` | Maximize active pane to fill tab (toggle) |
| `previous_tab` / `next_tab` / `last_tab` | Cycle between tabs |
| `goto_tab:1` .. `goto_tab:9` | Switch directly to tab 1 through 9 |
| `copy_to_clipboard` / `paste_from_clipboard` | Copy selection / Paste clipboard (bracketed-paste safe) |
| `select_all` | Select all scrollback buffer text |
| `clear_screen` | Clear scrollback history and redraw prompt at top |
| `start_search` / `search` | Open regex Find in Terminal overlay |
| `history_search` | Open zero-latency CLI fuzzy history search popup |
| `open_config` / `reload_config` | Open `~/.litty/config` in editor / Live-reload configuration |
| `jump_to_prompt:-1` / `jump_to_prompt:1` | Jump to previous / next command prompt mark (OSC 133) |
| `select_last_output` / `select_command_output` | Select and copy output of last completed command |
| `inspector` / `inspector:toggle` | Toggle terminal debug overlay |
| `toggle_secure_input` | Toggle macOS Secure Keyboard Entry |
| `check_for_updates` / `check_update` | Check for latest GitHub release |
| `text:<string>` | Send verbatim string / keystrokes to shell |
| `csi:<bytes>` / `esc:<bytes>` | Send raw CSI / Escape byte sequence |
| `unbind` / `ignore` | Disable default shortcut |
| `clear` | Drop all custom keybindings and restore defaults |

---

Back to the [README](../../README.md).
