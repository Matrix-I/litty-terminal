#!/usr/bin/env python3
"""Live GNOME Wayland clipboard, primary-selection and file-drop smoke test.

Requires /usr/bin/python3, PyGObject and GTK4. Opens a GTK peer and an isolated
Litty, sends keyboard/pointer input through Mutter's RemoteDesktop API, and
restores the prior text clipboard/primary content. Keep the screen unlocked
and leave keyboard/mouse idle while it runs. Non-text clipboard formats are
not preserved. This is an explicit live-desktop test, not a container test.
"""
import argparse
import os
import pathlib
import re
import shlex
import subprocess
import tempfile
import time
import urllib.parse

import gi
gi.require_version('Gtk', '4.0')
gi.require_version('Gdk', '4.0')
from gi.repository import Gdk, Gio, GLib, Gtk


def pump(seconds=.1):
    end = time.monotonic() + seconds
    context = GLib.MainContext.default()
    while time.monotonic() < end:
        while context.pending():
            context.iteration(False)
        time.sleep(.005)


def wait_for(read, condition, label, seconds=15):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        value = read()
        if condition(value):
            return value
        pump(.05)
    raise AssertionError(label)


class Input:
    def __init__(self):
        self.bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
        self.dest = 'org.gnome.Mutter.RemoteDesktop'
        self.path = self.bus.call_sync(
            self.dest, '/org/gnome/Mutter/RemoteDesktop', self.dest, 'CreateSession',
            None, None, Gio.DBusCallFlags.NONE, 5000, None).unpack()[0]
        self.call('Start')

    def call(self, method, signature=None, args=None):
        value = GLib.Variant(signature, args) if signature else None
        return self.bus.call_sync(self.dest, self.path, self.dest + '.Session', method,
                                  value, None, Gio.DBusCallFlags.NONE, 5000, None)

    def chord(self, modifiers, key):
        def send(code, down):
            self.call('NotifyKeyboardKeycode', '(ub)', (code, down))
            pump(.03)
        for modifier in modifiers:
            send(modifier, True)
        try:
            send(key, True)
            send(key, False)
        finally:
            for modifier in reversed(modifiers):
                send(modifier, False)
        pump(.3)

    def motion(self, x, y):
        self.call('NotifyPointerMotionRelative', '(dd)', (float(x), float(y)))
        pump(.06)

    def pointer(self, x, y):
        self.motion(-100000, -100000)
        self.motion(x, y)
        pump(.1)

    def button(self, code, down):
        self.call('NotifyPointerButton', '(ib)', (code, down))
        pump(.1)


def read_clipboard(clipboard):
    result = []
    def done(obj, response):
        try:
            result.append(obj.read_text_finish(response))
        except GLib.Error:
            result.append(None)
    clipboard.read_text_async(None, done)
    return wait_for(lambda: result, bool, 'clipboard read')[0]


def focus_peer(peer, control):
    peer.present()
    pump(.5)
    if not peer.is_active():
        control.chord([56], 15)  # Alt+Tab back from Litty.
    wait_for(peer.is_active, bool, 'GTK peer did not get focus')


def assert_litty_focus(trace):
    events = re.findall(r'zwp_text_input_v3#\d+\.(enter|leave)\(', trace())
    assert events and events[-1] == 'enter', 'Litty lost focus; refusing keyboard input'


def assert_litty_pointer(trace):
    events = re.findall(r'wl_pointer#\d+\.(enter|leave)\(', trace())
    assert events and events[-1] == 'enter', 'pointer is outside Litty; refusing click'


def clipboard_test(peer, control, clip, primary, trace, lines):
    assert_litty_focus(trace)
    control.chord([29, 42], 47)  # Ctrl+Shift+V.
    assert_litty_focus(trace)
    control.chord([], 28)
    wait_for(lines, lambda l: len(l) >= 1, 'clipboard paste')
    assert lines()[0] == 'GTK clipboard tiếng', 'unexpected clipboard paste'
    print('PASS GTK -> Litty clipboard', flush=True)

    control.pointer(300, 200)
    motions = re.findall(r'wl_pointer#\d+\.motion\(\d+, ([\d.]+), ([\d.]+)\)', trace())
    assert motions, 'no pointer position on Litty'
    # Account for GNOME's dock and top bar from the actual surface coordinates.
    local_x, local_y = map(float, motions[-1])
    origin = (300 - local_x, 200 - local_y)
    assert_litty_pointer(trace)
    control.button(274, True)
    control.button(274, False)
    assert_litty_focus(trace)
    control.chord([], 28)
    wait_for(lines, lambda l: len(l) >= 2, 'primary paste')
    assert lines()[1] == 'GTK primary Việt', 'unexpected primary paste'
    print('PASS GTK -> Litty primary selection', flush=True)

    assert_litty_focus(trace)
    control.chord([29, 42], 30)  # Select all.
    control.chord([29, 42], 46)  # Copy.
    control.chord([56], 15)
    wait_for(peer.is_active, bool, 'GTK clipboard consumer focus')
    copied = read_clipboard(clip)
    assert copied and 'LITTY COPY MARKER' in copied, 'unexpected copied text'
    print('PASS Litty -> GTK clipboard', flush=True)

    control.chord([56], 15)
    assert_litty_focus(trace)
    rect = re.search(r'set_cursor_rectangle\((\d+), (\d+), (\d+), (\d+)\)', trace())
    x, y, width, height = map(int, rect.groups())
    # The probe printed one line, so its first cursor rectangle is on row 1.
    control.pointer(origin[0] + x + width, origin[1] + y - height / 2)
    control.button(272, True)
    try:
        control.motion(width * 20, 0)
        pump(.2)
    finally:
        control.button(272, False)
    control.chord([56], 15)
    wait_for(peer.is_active, bool, 'GTK primary consumer focus')
    copied = read_clipboard(primary)
    assert copied and 'COPY' in copied, 'unexpected primary selection'
    print('PASS Litty -> GTK primary selection', flush=True)


def drop_test(peer, label, display, control, root, trace, lines):
    focus_peer(peer, control)
    peer.unmaximize()
    pump(.5)
    paths = [root / 'drop file.txt', root / "it's Việt.txt"]
    for path in paths:
        path.write_text('drop test')
    payload = ('\r\n'.join('file://' + urllib.parse.quote(str(p)) for p in paths) + '\r\n').encode()
    source = Gtk.DragSource.new()
    source.set_actions(Gdk.DragAction.COPY)
    source.connect('prepare', lambda *args: Gdk.ContentProvider.new_for_bytes('text/uri-list', GLib.Bytes.new(payload)))
    ended = []
    source.connect('drag-end', lambda obj, drag, delete: ended.append(delete))
    label.add_controller(source)
    geometry = display.get_monitors().get_item(0).get_geometry()
    pointer = display.get_default_seat().get_pointer()
    origin = None
    # Wayland deliberately has no global window-position API. Locate our GTK
    # surface using pointer focus, rather than assuming GNOME centered it.
    for y in range(100, geometry.height, 150):
        for x in range(120, geometry.width, 220):
            control.pointer(x, y)
            surface, local_x, local_y = pointer.get_surface_at_position()
            if surface == peer.get_surface():
                origin = (x - local_x, y - local_y)
                break
        if origin:
            break
    assert origin, 'could not locate GTK source on the first monitor'
    sx, sy = origin[0] + peer.get_width() / 2, origin[1] + peer.get_height() / 2
    tx, ty = geometry.width - 100, geometry.height / 2
    assert tx > origin[0] + peer.get_width(), 'no visible Litty area right of GTK source'

    for attempt in range(2):
        focus_peer(peer, control)
        before = len(lines())
        control.pointer(sx, sy)
        control.button(272, True)
        try:
            for _ in range(20):
                control.motion((tx - sx) / 20, (ty - sy) / 20)
        finally:
            control.button(272, False)
        wait_for(lambda: ended, lambda e: len(e) > attempt, 'DnD source completion')
        assert ended[-1] is False, 'the source must retain its files (Copy)'
        control.pointer(tx, ty)
        assert_litty_pointer(trace)
        control.button(272, True)
        control.button(272, False)
        assert_litty_focus(trace)
        control.chord([], 28)
        wait_for(lines, lambda l: len(l) > before, 'dropped paths at PTY')
        assert shlex.split(lines()[-1]) == list(map(str, paths)), lines()
    assert len(re.findall(r'wl_data_offer#\d+\.finish\(\)', trace())) >= 2
    print('PASS two consecutive multi-file drops, shell quoting and Copy completion', flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', nargs='?', default='target/debug/litty')
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument('--clipboard-only', action='store_true')
    mode.add_argument('--drop-only', action='store_true')
    args = parser.parse_args()
    binary = str(pathlib.Path(args.binary).resolve())
    control = Input()
    Gdk.set_allowed_backends("wayland")
    Gtk.init()
    peer = Gtk.Window(title='Litty Wayland data test')
    peer.set_default_size(450, 260)
    label = Gtk.Label(label='Wayland clipboard and file-drop source')
    label.set_hexpand(True)
    label.set_vexpand(True)
    peer.set_child(label)
    display = Gdk.Display.get_default()
    print("GTK peer backend:", display.__gtype__.name, flush=True)
    clip, primary = display.get_clipboard(), display.get_primary_clipboard()
    saved = []
    app = None
    try:
        if not args.drop_only:
            peer.present()
            pump(.8)
            for _ in range(10):
                if peer.is_active():
                    break
                control.chord([56], 1)  # Alt+Escape cycles windows without typing.
            assert peer.is_active(), 'GTK producer did not get focus'
            control.chord([], 42)  # Give GTK a valid input serial for selection ownership.
            saved = [(clip, read_clipboard(clip)), (primary, read_clipboard(primary))]
            clip.set('GTK clipboard tiếng')
            primary.set('GTK primary Việt')
            pump(.3)
        with tempfile.TemporaryDirectory(prefix='litty-wayland-data-') as tmp:
            root = pathlib.Path(tmp)
            (root / '.litty').mkdir()
            (root / '.litty/config').write_text('auto-update = off\nshell-integration = none\nconfirm-close-surface = false\n')
            shell = root / 'shell'
            shell.write_text(f'''#!/bin/bash
printf 'LITTY COPY MARKER\\n'
while [ ! -e '{root / 'quit'}' ]; do
    IFS= read -r -t 0.2 line && printf '%s\\n' "$line" >> '{root / 'typed'}'
done
''')
            shell.chmod(0o700)
            logpath = root / 'protocol'
            def trace():
                return logpath.read_text(errors='replace')
            def lines():
                return (root / 'typed').read_text().splitlines() if (root / 'typed').exists() else []
            with logpath.open('w') as log:
                env = dict(os.environ, HOME=tmp, SHELL=str(shell), LITTY_BACKEND='wayland', WAYLAND_DEBUG='1')
                app = subprocess.Popen([binary], env=env, stdout=subprocess.DEVNULL, stderr=log)
                try:
                    wait_for(trace, lambda t: re.search(r'zwp_text_input_v3#\d+\.enable\(', t), 'Litty focus')
                    assert_litty_focus(trace)
                    control.chord([125], 103)  # Super+Up maximizes the destination.
                    if not args.drop_only:
                        clipboard_test(peer, control, clip, primary, trace, lines)
                    if not args.clipboard_only:
                        drop_test(peer, label, display, control, root, trace, lines)
                    (root / 'quit').touch()
                    assert app.wait(timeout=10) == 0
                    assert not re.search(r'wl_display#\d+\.error\(', trace())
                    assert 'opening an X11 window instead' not in trace()
                    print('PASS clean native Wayland data session', flush=True)
                finally:
                    with tempfile.NamedTemporaryFile(prefix='litty-wayland-data-', suffix='.log', delete=False) as output:
                        output.write(logpath.read_bytes())
                        print('Protocol trace:', output.name, flush=True)
    finally:
        if app is not None and app.poll() is None:
            app.terminate()
            app.wait(timeout=5)
        if saved:
            focus_peer(peer, control)
            control.chord([], 42)
        for clipboard, text in saved:
            if text is not None:
                clipboard.set(text)
        pump(.3)
        peer.close()
        control.call('Stop')


if __name__ == '__main__':
    main()
