//! Terminal inspector overlay text.

use super::pointer::pane_cell_at_mouse;
use super::state::State;

/// What the terminal inspector shows about the active pane.
pub(super) fn inspector_lines(s: &mut State) -> Vec<String> {
    let mouse_cell = pane_cell_at_mouse(s);
    let Some(tab) = s.tabs.get_mut(s.active) else { return Vec::new() };
    let Some(pane) = tab.active_pane_mut() else { return Vec::new() };
    let mouse_cell = mouse_cell.filter(|&(id, _, _)| id == pane.id);
    let running = pane.has_running_process();
    let cwd = pane.cwd().unwrap_or_default();
    let g = pane.grid.lock();
    let on = |b: bool| if b { "on" } else { "off" };
    let hex = |c: u32| format!("#{:06x}", c >> 8);
    let (history, offset) = g.scroll_position();
    let cursor = g.cursor_pos().map_or("hidden".to_string(), |(c, r)| format!("row {r} col {c}"));
    let shape = match g.cursor_shape {
        0 => "default",
        1 | 2 => "block",
        3 | 4 => "underline",
        _ => "bar",
    };
    let cell = mouse_cell
        .and_then(|(_, r, c)| g.describe_cell(r, c).map(|d| format!("{r},{c}  {d}")))
        .unwrap_or_else(|| "(move the mouse over this pane)".to_string());
    vec![
        "Terminal Inspector".to_string(),
        format!("Grid      {}x{}  scrollback {history} lines, {offset} up", g.cols, g.rows),
        format!("Cursor    {cursor}  {shape}  {}", if g.cursor_visible { "visible" } else { "invisible" }),
        format!(
            "Screen    {}  app-cursor {}  bracketed-paste {}  sync {}",
            if g.alt_active { "alternate" } else { "main" },
            on(g.app_cursor),
            on(g.bracketed_paste),
            on(g.sync_output)
        ),
        format!(
            "Mouse     {:?}  sgr {}  pixels {}  focus {}  alt-scroll {}",
            g.mouse_tracking,
            on(g.mouse_sgr),
            on(g.mouse_sgr_pixels),
            on(g.focus_events),
            on(g.alt_scroll)
        ),
        format!("Keyboard  kitty flags {}  modifyOtherKeys {}", g.kitty_keyboard_flags(), g.modify_other_keys),
        format!("Colors    fg {}  bg {}", hex(g.fg_color()), hex(g.bg_color())),
        format!("Process   {}  cwd {cwd}", if running { "running" } else { "shell at prompt" }),
        format!("Title     {:?}", g.title),
        format!("Cell      {cell}"),
        format!("Last key  {}", s.inspector.last_key),
    ]
}
