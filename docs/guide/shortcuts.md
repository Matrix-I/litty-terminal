# Default Shortcuts

Litty provides complete shortcut parity across macOS and Linux, adhering strictly to terminal conventions on each platform:

| Action | macOS (`Cmd`) | Linux (`Ctrl` / `Super`) | Notes |
| :--- | :--- | :--- | :--- |
| **New tab** *(inherits active CWD)* | <kbd>Cmd</kbd> + <kbd>T</kbd> | <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>T</kbd> *(or <kbd>Super</kbd> + <kbd>T</kbd>)* | Preserves bare `Ctrl+T` (transpose/fzf) |
| **New window** *(inherits active CWD)* | <kbd>Cmd</kbd> + <kbd>N</kbd> | <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>N</kbd> *(or <kbd>Super</kbd> + <kbd>N</kbd>)* | Preserves bare `Ctrl+N` (history) |
| **Close active pane / tab** | <kbd>Cmd</kbd> + <kbd>W</kbd> | <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>W</kbd> *(or <kbd>Super</kbd> + <kbd>W</kbd>)* | Closes active split pane; closes tab if last pane remaining |
| **Split pane right (Vertical)** | <kbd>Cmd</kbd> + <kbd>D</kbd> | <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>D</kbd> *(or <kbd>Super</kbd> + <kbd>D</kbd>)* | Splits active pane vertically with hairline divider |
| **Split pane down (Horizontal)** | <kbd>Cmd</kbd> + <kbd>Shift</kbd> + <kbd>D</kbd> | <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>E</kbd> / <kbd>Super</kbd> + <kbd>Shift</kbd> + <kbd>D</kbd> | Splits active pane horizontally |
| **Toggle split zoom** | <kbd>Cmd</kbd> + <kbd>Shift</kbd> + <kbd>Enter</kbd> | <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>Enter</kbd> *(or <kbd>Super</kbd> + <kbd>Shift</kbd> + <kbd>Enter</kbd>)* | Expands active split to fill window; press again to restore |
| **Equalize split sizes** | <kbd>Cmd</kbd> + <kbd>Option</kbd> + <kbd>=</kbd> | <kbd>Ctrl</kbd> + <kbd>Alt</kbd> + <kbd>=</kbd> *(or <kbd>Super</kbd> + <kbd>Alt</kbd> + <kbd>=</kbd>)* | Resets all split dividers in current tab to equal ratios |
| **Navigate splits / tabs** | <kbd>Cmd</kbd> + <kbd>←</kbd> / <kbd>→</kbd> / <kbd>↑</kbd> / <kbd>↓</kbd> *(or + <kbd>Option</kbd>)* | <kbd>Ctrl</kbd> + <kbd>Alt</kbd> + Arrows *(or <kbd>Super</kbd> + Arrows)* | Spatial focus navigation (falls back to cycling tabs at edge) |
| **Cycle splits (Prev / Next)** | <kbd>Cmd</kbd> + <kbd>[</kbd> / <kbd>]</kbd> | <kbd>Ctrl</kbd> + <kbd>Alt</kbd> + <kbd>[</kbd> / <kbd>]</kbd> *(or <kbd>Super</kbd> + <kbd>[</kbd> / <kbd>]</kbd>)* | Cycles focus across split panes in active tab |
| **Resize active split** | <kbd>Cmd</kbd> + <kbd>Ctrl</kbd> + Arrows | <kbd>Super</kbd> + <kbd>Ctrl</kbd> + Arrows / <kbd>Ctrl</kbd> + <kbd>Alt</kbd> + <kbd>Shift</kbd> + Arrows | Dynamically expands or shrinks active split pane |
| **Quit Litty** | <kbd>Cmd</kbd> + <kbd>Q</kbd> | <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>Q</kbd> *(or <kbd>Super</kbd> + <kbd>Q</kbd>)* | Preserves bare `Ctrl+Q` (XON flow control) |
| **Minimize window** | <kbd>Cmd</kbd> + <kbd>M</kbd> | <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>M</kbd> *(or <kbd>Super</kbd> + <kbd>M</kbd>)* | Preserves bare `Ctrl+M` (Enter / Carriage return) |
| **Cycle tabs (Prev / Next)** | <kbd>Cmd</kbd> + <kbd>Shift</kbd> + <kbd>[</kbd> / <kbd>]</kbd> | <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>[</kbd> / <kbd>]</kbd> | Circular tab switching |
| **Cycle tabs (Next / Prev)** | <kbd>Ctrl</kbd> + <kbd>Tab</kbd> / <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>Tab</kbd> | <kbd>Ctrl</kbd> + <kbd>Tab</kbd> / <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>Tab</kbd> | Cross-platform standard |
| **Navigate tabs (Left / Right)** | <kbd>Cmd</kbd> + <kbd>Option</kbd> + <kbd>←</kbd> / <kbd>→</kbd> | <kbd>Ctrl</kbd> + <kbd>Alt</kbd> + <kbd>←</kbd> / <kbd>→</kbd> *(or <kbd>Super</kbd> + <kbd>Alt</kbd>)* | Direct spatial switching |
| **Jump to tab 1..9** | <kbd>Cmd</kbd> + <kbd>1</kbd> .. <kbd>9</kbd> | <kbd>Ctrl</kbd> + <kbd>1</kbd> .. <kbd>9</kbd>, <kbd>Alt</kbd> + <kbd>1</kbd> .. <kbd>9</kbd> | Matches `^1`..`^9` badges; `9` jumps to last tab |
| **Copy selection** | <kbd>Cmd</kbd> + <kbd>C</kbd> | <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>C</kbd> *(or <kbd>Super</kbd> + <kbd>C</kbd>, <kbd>Ctrl</kbd> + <kbd>Insert</kbd>)* | Preserves `Ctrl+C` (SIGINT) |
| **Paste clipboard** | <kbd>Cmd</kbd> + <kbd>V</kbd> | <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>V</kbd> *(or <kbd>Super</kbd> + <kbd>V</kbd>, <kbd>Shift</kbd> + <kbd>Insert</kbd>)* | Bracketed paste aware; preserves `Ctrl+V` (literal escape). On Linux, selecting text also sets the PRIMARY selection and middle-click pastes it |
| **Zoom in (+1.0 pt)** | <kbd>Cmd</kbd> + <kbd>+</kbd> / <kbd>=</kbd> | <kbd>Ctrl</kbd> + <kbd>+</kbd> / <kbd>=</kbd> *(with/without Shift, or Super)* | Font size clamps between 6.0 – 72.0 pt |
| **Zoom out (-1.0 pt)** | <kbd>Cmd</kbd> + <kbd>-</kbd> | <kbd>Ctrl</kbd> + <kbd>-</kbd> / <kbd>_</kbd> *(with/without Shift, or Super)* | Font size clamps between 6.0 – 72.0 pt |
| **Reset zoom** | <kbd>Cmd</kbd> + <kbd>0</kbd> | <kbd>Ctrl</kbd> + <kbd>0</kbd> *(with/without Shift, or Super)* | Resets to configured default font size |
| **Open config editor** | <kbd>Cmd</kbd> + <kbd>,</kbd> | <kbd>Ctrl</kbd> + <kbd>,</kbd> *(or <kbd>Super</kbd> + <kbd>,</kbd>)* | Opens `~/.litty/config` in `$EDITOR` |
| **Reload config file** | <kbd>Cmd</kbd> + <kbd>Shift</kbd> + <kbd>,</kbd> | <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>,</kbd> | Live reload without losing shell session |
| **Open URL or path** | <kbd>Cmd</kbd> + <kbd>Click</kbd> | <kbd>Ctrl</kbd> + <kbd>Click</kbd> *(or <kbd>Super</kbd> + <kbd>Click</kbd>)* | Opens in default browser / editor via `open` or `xdg-open` |
| **Hover URL preview** | <kbd>Cmd</kbd> + <kbd>Mouse Move</kbd> | <kbd>Ctrl</kbd> + <kbd>Mouse Move</kbd> *(or <kbd>Super</kbd>)* | Underlines URL and displays preview pill |
| **Jump to previous / next prompt** | <kbd>Cmd</kbd> + <kbd>Shift</kbd> + <kbd>↑</kbd> / <kbd>↓</kbd> | <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>↑</kbd> / <kbd>↓</kbd> | Scrolls between shell prompts (needs shell integration) |
| **Select last command output** | <kbd>Cmd</kbd> + <kbd>Shift</kbd> + <kbd>O</kbd> | <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>O</kbd> | Selects and copies the output of the last command (needs shell integration) |
| **Terminal inspector** | <kbd>Cmd</kbd> + <kbd>Option</kbd> + <kbd>I</kbd> | <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>I</kbd> *(or <kbd>Super</kbd> + <kbd>Alt</kbd> + <kbd>I</kbd>)* | Debug overlay: modes, protocols, cell under the mouse, last key's bytes |
| **Select all** | <kbd>Cmd</kbd> + <kbd>A</kbd> | <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>A</kbd> *(or <kbd>Super</kbd> + <kbd>A</kbd>)* | Selects all scrollback and screen text; copy with <kbd>Cmd</kbd>/<kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>C</kbd>. Preserves bare `Ctrl+A` (beginning of line) |
| **Extend selection** | <kbd>Shift</kbd> + <kbd>Click</kbd> / <kbd>Drag</kbd> | <kbd>Shift</kbd> + <kbd>Click</kbd> / <kbd>Drag</kbd> | Moves the nearest end of the current selection to the mouse and copies it |
| **Select text in mouse apps** | <kbd>Shift</kbd> + <kbd>Drag</kbd> | <kbd>Shift</kbd> + <kbd>Drag</kbd> | Bypasses mouse reporting in vim/tmux/htop to select and copy locally |
| **Click to position cursor** | <kbd>Mouse Left Click</kbd> | <kbd>Mouse Left Click</kbd> | Repositions cursor to clicked character on active command line (Unicode & wide-char aligned) |
| **Jump word backward / forward** | <kbd>Option</kbd> + <kbd>←</kbd> / <kbd>→</kbd> | <kbd>Ctrl</kbd> + <kbd>←</kbd> / <kbd>→</kbd> *(or <kbd>Alt</kbd> + Arrows)* | Jumps cursor backward/forward by word (`\x1bb` / `\x1bf`). On macOS, strictly `Option+Arrows` to avoid Mission Control Spaces collision |
| **Delete before cursor** | <kbd>Ctrl</kbd> + <kbd>U</kbd> | <kbd>Ctrl</kbd> + <kbd>U</kbd> | Prompt-aware deletion from prompt to cursor (preserves prompt & text after cursor; preserves Vim scroll in alt screen) |
| **Delete after cursor** | <kbd>Ctrl</kbd> + <kbd>K</kbd> | <kbd>Ctrl</kbd> + <kbd>K</kbd> | Deletes all text from cursor to end of command line (`kill-line`) |
| **Clear screen & scrollback** | <kbd>Cmd</kbd> + <kbd>K</kbd> | <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>K</kbd> | Clears scrollback history buffer and redraws prompt at top (matches Terminal.app / iTerm2 / Ghostty) |
| **CLI History Search** | <kbd>Ctrl</kbd> + <kbd>R</kbd> *(or <kbd>Cmd</kbd> + <kbd>R</kbd>)* | <kbd>Ctrl</kbd> + <kbd>R</kbd> *(or <kbd>Super</kbd> + <kbd>R</kbd> / <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>R</kbd>)* | Built-in fuzzy history search; `Enter`: execute, `Tab`: insert, `Esc`: close; preserves vim redo |
| **Find in Terminal** | <kbd>Cmd</kbd> + <kbd>F</kbd> | <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>F</kbd> *(or <kbd>Super</kbd> + <kbd>F</kbd>)* | Regex search box (top-right); `Enter`/`↑`: older match, `Shift+Enter`/`↓`: newer, `Ctrl+R`: toggle regex/literal, `Esc`: close |
| **Scroll history locally** | <kbd>Shift</kbd> + <kbd>PageUp</kbd> / <kbd>PageDown</kbd> | <kbd>Shift</kbd> + <kbd>PageUp</kbd> / <kbd>PageDown</kbd> | Scrolls active tab's scrollback buffer |

---

Back to the [README](../../README.md).
