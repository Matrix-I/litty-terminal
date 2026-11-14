#!/usr/bin/env python3
"""Check native text-input-v3 on a live GNOME Wayland desktop.

Default: open an isolated Litty window for four seconds and check binding,
focus, terminal purpose and candidate rectangle. With --telex, use Mutter's
RemoteDesktop keyboard API and IBus Unikey to check that `tieengs vieetj`
arrives at the PTY as `tiếng việt`. Requires /usr/bin/python3 with PyGObject
and the IBus typelib. Temporarily selects Unikey and restores the old engine;
only injects keys while the test window has text-input focus. Does not check
candidate popup appearance. Run this live-desktop test explicitly.
"""
import argparse
import os
import pathlib
import re
import subprocess
import tempfile
import time


def wait_for(read, condition, label, seconds=15):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = read()
        if condition(value):
            return value
        time.sleep(.1)
    raise AssertionError(label)


class Keyboard:
    def __init__(self):
        import gi
        gi.require_version('IBus', '1.0')
        from gi.repository import Gio, GLib, IBus
        self.gio, self.glib = Gio, GLib
        self.bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
        self.dest = 'org.gnome.Mutter.RemoteDesktop'
        IBus.init()
        self.ibus = IBus.Bus.new()
        engine = self.ibus.get_global_engine()
        assert engine is not None, 'IBus has no current engine'
        self.previous = engine.get_name()
        self.path = self.bus.call_sync(
            self.dest, '/org/gnome/Mutter/RemoteDesktop', self.dest, 'CreateSession',
            None, None, Gio.DBusCallFlags.NONE, 5000, None).unpack()[0]
        try:
            self.call('Start')
        except Exception:
            self.call('Stop')
            raise

    def call(self, method, args=None):
        return self.bus.call_sync(self.dest, self.path, self.dest + '.Session', method,
                                  args, None, self.gio.DBusCallFlags.NONE, 5000, None)

    def type_telex(self, trace):
        assert self.ibus.set_global_engine('Unikey'), 'cannot select IBus Unikey'
        time.sleep(.5)
        for sym in [*map(ord, 'tieengs vieetj'), 0xff0d]:
            focus = re.findall(r'zwp_text_input_v3#\d+\.(enter|leave)\(', trace())
            assert focus and focus[-1] == 'enter', 'test window lost focus; refusing further input'
            self.call('NotifyKeyboardKeysym', self.glib.Variant('(ub)', (sym, True)))
            try:
                time.sleep(.04)
            finally:
                self.call('NotifyKeyboardKeysym', self.glib.Variant('(ub)', (sym, False)))
            time.sleep(.08)

    def close(self):
        try:
            assert self.ibus.set_global_engine(self.previous), 'cannot restore IBus engine'
        finally:
            self.call('Stop')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', nargs='?', default='target/debug/litty')
    parser.add_argument('--telex', action='store_true', help='inject Telex through GNOME and IBus Unikey')
    args = parser.parse_args()
    binary = str(pathlib.Path(args.binary).resolve())
    keyboard = Keyboard() if args.telex else None
    app = None
    try:
        with tempfile.TemporaryDirectory(prefix='litty-wayland-ime-') as tmp:
            root = pathlib.Path(tmp)
            (root / '.litty').mkdir()
            (root / '.litty/config').write_text(
                'auto-update = off\nshell-integration = none\nconfirm-close-surface = false\n')
            shell = root / 'shell'
            body = ('IFS= read -r -t 20 line\nprintf "%s\\n" "$line" > "' + str(root / 'typed') + '"\nsleep 1\n'
                    if args.telex else 'sleep 4\n')
            shell.write_text('#!/bin/bash\n' + body)
            shell.chmod(0o700)
            logpath = root / 'protocol'

            def trace():
                return logpath.read_text(errors='replace')

            with logpath.open('w') as log:
                env = dict(os.environ, HOME=tmp, SHELL=str(shell), LITTY_BACKEND='wayland', WAYLAND_DEBUG='1')
                app = subprocess.Popen([binary], env=env, stdout=subprocess.DEVNULL, stderr=log)
                try:
                    wait_for(trace, lambda t: re.search(r'zwp_text_input_v3#\d+\.enable\(', t), 'text input not enabled')
                    if keyboard:
                        keyboard.type_telex(trace)
                    assert app.wait(timeout=25) == 0, 'Litty failed'
                    text = trace()
                    assert re.search(r'zwp_text_input_manager_v3#\d+\.get_text_input\(', text)
                    assert re.search(r'zwp_text_input_v3#\d+\.set_content_type\(0, 13\)', text)
                    assert re.search(r'zwp_text_input_v3#\d+\.set_cursor_rectangle\(', text)
                    assert re.search(r'zwp_text_input_v3#\d+\.done\(', text)
                    assert 'opening an X11 window instead' not in text
                    assert not re.search(r'wl_display#\d+\.error\(', text)
                    if keyboard:
                        typed = (root / 'typed').read_text()
                        assert typed == 'tiếng việt\n', repr(typed)
                        assert '.preedit_string(' in text and '.commit_string(' in text
                        print('PASS Unikey Telex commits tiếng việt through native GNOME text-input-v3')
                    print('PASS text-input-v3 focus, terminal purpose, candidate rectangle and clean exit')
                finally:
                    # Keep a unique trace outside the temporary HOME, including on failure.
                    with tempfile.NamedTemporaryFile(prefix='litty-wayland-ime-', suffix='.log', delete=False) as saved:
                        saved.write(logpath.read_bytes())
                        print('Protocol trace:', saved.name, flush=True)
    finally:
        try:
            if keyboard:
                keyboard.close()
        finally:
            if app is not None and app.poll() is None:
                app.terminate()
                app.wait(timeout=5)


if __name__ == '__main__':
    main()
