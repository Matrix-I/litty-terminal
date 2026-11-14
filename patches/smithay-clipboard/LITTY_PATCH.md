# Litty's smithay-clipboard 0.7.3 patch

The upstream 0.7.3 source and MIT license are preserved here. The only API
extension is `Clipboard::new_with_dnd`, which optionally negotiates Copy for
`text/uri-list` on the existing data device and hands a dropped offer to a
nonblocking callback. The receiver owns finishing/destroying that offer.
The clipboard's store/load and primary-selection paths are unchanged.

Reason: GNOME routes selection/drag events to the most recently bound
`wl_data_device` for a client and seat. A separate Litty drop device on the
same connection loses events to the clipboard's device (observed in the
Wayland protocol trace), so both features must use the same one.

Changes are confined to `src/lib.rs`, `src/worker.rs`, and `src/state.rs`.
Litty's bounded pipe reads, URI validation, window routing, and delivery
remain in `src/platform/linux/native/wayland_data.rs` and `wayland.rs`.

When updating the upstream version, reapply this hook or use an upstream
replacement, then run `scripts/linux/wayland_data_smoke.py` in a live GNOME
session to check clipboard and primary selection both ways, consecutive
file drops, and protocol cleanup. The script uses GTK4 and Mutter's test
input API, restores prior text clipboard content, and needs an unlocked
screen with the keyboard/mouse idle.
