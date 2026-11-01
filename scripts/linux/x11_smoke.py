#!/usr/bin/env python3
"""Synthetic Linux desktop smoke checks under Xvfb + Openbox.

Needs the Linux container image and a compiled litty binary. This checks X11
software behavior, not physical lock keys, Wayland, or hardware GPU latency.
"""
import ctypes
import json
import os
import pathlib
import re
import subprocess
import sys
import tempfile
import time


def run(*args, check=True, env=None):
    return subprocess.run(args, check=check, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, text=True, timeout=10, env=env)


def wait_for(read, condition, label, seconds=15):
    deadline = time.monotonic() + seconds
    last = None
    while time.monotonic() < deadline:
        last = read()
        if condition(last):
            return last
        time.sleep(.1)
    raise AssertionError(f"{label}: {last!r}")


class Keyboard:
    """Raw XTEST keycodes: xdotool's keysym synthesis can change Num Lock."""
    def __init__(self):
        self.x = ctypes.CDLL('libX11.so.6')
        self.xt = ctypes.CDLL('libXtst.so.6')
        self.x.XOpenDisplay.restype = ctypes.c_void_p
        self.x.XOpenDisplay.argtypes = [ctypes.c_char_p]
        self.x.XStringToKeysym.restype = ctypes.c_ulong
        self.x.XStringToKeysym.argtypes = [ctypes.c_char_p]
        self.x.XKeysymToKeycode.restype = ctypes.c_ubyte
        self.x.XKeysymToKeycode.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
        self.x.XkbLockModifiers.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_uint, ctypes.c_uint]
        self.x.XSync.argtypes = [ctypes.c_void_p, ctypes.c_int]
        self.x.XCloseDisplay.argtypes = [ctypes.c_void_p]
        self.xt.XTestFakeKeyEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
        self.display = self.x.XOpenDisplay(None)
        assert self.display, 'XOpenDisplay'
        assert self.x.XkbLockModifiers(self.display, 0x100, 2 | 16, 0), 'reset Caps/Num Lock'
        self.x.XSync(self.display, 0)

    def press(self, symbol):
        code = self.x.XKeysymToKeycode(self.display, self.x.XStringToKeysym(symbol.encode()))
        assert code, symbol
        for down in [1, 0]:
            assert self.xt.XTestFakeKeyEvent(self.display, code, down, 0)
        self.x.XSync(self.display, 0)

    def chord(self, *symbols):
        codes = [self.x.XKeysymToKeycode(self.display, self.x.XStringToKeysym(s.encode())) for s in symbols]
        assert all(codes), symbols
        for code in codes:
            assert self.xt.XTestFakeKeyEvent(self.display, code, 1, 0)
        for code in reversed(codes):
            assert self.xt.XTestFakeKeyEvent(self.display, code, 0, 0)
        self.x.XSync(self.display, 0)

    def close(self):
        self.x.XCloseDisplay(self.display)


def main():
    binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else 'target/debug/litty').resolve())
    processes = []
    keyboard = Keyboard()
    with tempfile.TemporaryDirectory(prefix='litty-x11-') as temporary:
        root = pathlib.Path(temporary)
        home = root / 'home'
        (home / '.litty').mkdir(parents=True)
        (home / '.litty/config').write_text(
            'auto-update = off\nshell-integration = none\nconfirm-close-surface = false\n'
            'window-padding-x = 8\nwindow-padding-y = 8\ntitle = Litty Linux probe\n'
            'keybind = global:ctrl+grave_accent=toggle_quick_terminal\n')
        runtime = root / 'runtime'
        runtime.mkdir(mode=0o700)
        events = root / 'keys.jsonl'
        shell = root / 'probe-shell'
        shell.write_text('''#!/usr/bin/python3
import json,os,tty
fd=os.open(os.environ['LITTY_PROBE_EVENTS'],os.O_WRONLY|os.O_CREAT|os.O_APPEND,0o600)
tty.setraw(0)
os.write(fd,(json.dumps({'ready':os.getpid()})+'\\n').encode())
os.write(1,b'\\x1b[2J\\x1b[H\\x1b[16t\\x1b[>9u\\x1b[6n')
while True:
    data=os.read(0,4096)
    if not data: break
    os.write(fd,(json.dumps({'pid':os.getpid(),'hex':data.hex()})+'\\n').encode())
''')
        shell.chmod(0o700)
        env = dict(os.environ, HOME=str(home), SHELL=str(shell),
                   LITTY_PROBE_EVENTS=str(events), XDG_RUNTIME_DIR=str(runtime), WINIT_UNIX_BACKEND='x11')
        env.pop('WAYLAND_DISPLAY', None)
        log = (root / 'litty.log').open('w+')
        wm = subprocess.Popen(['openbox'], stdout=log, stderr=log, env=env)
        processes.append(wm)
        time.sleep(.5)
        app = subprocess.Popen([binary], stdout=log, stderr=log, env=env)
        processes.append(app)

        def records():
            if not events.exists():
                return []
            return [json.loads(line) for line in events.read_text().splitlines()]

        def visible():
            result = run('xdotool', 'search', '--onlyvisible', '--pid', str(app.pid), check=False)
            return result.stdout.split()

        try:
            wait_for(records, lambda r: any('ready' in x for x in r), 'shell start')
            wait_for(records, lambda r: any('hex' in x and b'R' in bytes.fromhex(x['hex']) for x in r), 'Kitty mode acknowledgement')
            normal = wait_for(visible, lambda v: len(v) == 1, 'normal window')[0]
            run('xdotool', 'windowactivate', '--sync', normal)

            def press(*keys):
                start = len(records())
                if len(keys) == 1 and keys[0] in ['Caps_Lock', 'Num_Lock', 'KP_7', 'KP_Home']:
                    keyboard.press(keys[0])
                else:
                    run('xdotool', 'key', '--delay', '100', *keys)
                time.sleep(.25)
                return b''.join(bytes.fromhex(x['hex']) for x in records()[start:] if 'hex' in x)

            def mods(data, code):
                found = re.findall(rb'\x1b\[' + str(code).encode() + rb'(?:;(\d+))?u', data)
                assert found, (code, data)
                return int(found[-1] or b'1') - 1

            assert mods(press('ctrl+a'), 97) == 4
            press('Caps_Lock')
            assert mods(press('ctrl+a'), 97) == 4 + 64
            press('Caps_Lock')
            assert mods(press('ctrl+a'), 97) == 4
            press('Num_Lock')
            keypad = press('KP_7')
            assert mods(keypad, 57406) & 128, keypad
            press('Num_Lock')
            keypad = press('KP_Home')
            assert mods(keypad, 57423) & 128 == 0, keypad
            print('PASS X11 synthetic Caps/Num Lock Kitty modifiers', flush=True)

            external = subprocess.Popen(['xterm', '-fa', 'DejaVu Sans Mono', '-fs', '12', '-title', 'External probe'],
                                        stdout=log, stderr=log, env=dict(env, SHELL='/bin/sh'))
            processes.append(external)
            other = wait_for(lambda: run('xdotool', 'search', '--onlyvisible', '--pid', str(external.pid), check=False).stdout.split(), bool, 'external window')[0]
            run('xdotool', 'windowactivate', '--sync', other)
            keyboard.chord('Control_L', 'grave')
            shown = wait_for(visible, lambda v: len(v) == 2, 'global hotkey shows Quick Terminal')
            quick = next(w for w in shown if w != normal)
            geometry = run('xdotool', 'getwindowgeometry', '--shell', quick).stdout
            values = dict(re.findall(r'^(\w+)=(\d+)$', geometry, re.M))
            assert int(values['WIDTH']) >= 1200 and 200 <= int(values['HEIGHT']) <= 500, geometry
            wait_for(lambda: run('xdotool', 'getactivewindow').stdout.strip(), lambda w: w == quick, 'Quick Terminal takes focus')
            ready = [x['ready'] for x in wait_for(records, lambda r: sum('ready' in x for x in r) == 2, 'quick shell') if 'ready' in x]
            keyboard.chord('Control_L', 'grave')
            wait_for(visible, lambda v: v == [normal], 'global hotkey hides Quick Terminal')
            run(binary, '+toggle-quick-terminal', env=env)
            wait_for(visible, lambda v: quick in v and len(v) == 2, 'socket restores same Quick Terminal')
            wait_for(lambda: run('xdotool', 'getactivewindow').stdout.strip(), lambda w: w == quick, 'restored panel takes focus')
            assert [x['ready'] for x in records() if 'ready' in x] == ready
            run('xdotool', 'windowactivate', '--sync', other)
            wait_for(visible, lambda v: v == [normal], 'Quick Terminal autohides')
            print('PASS X11 Quick Terminal global hotkey, geometry, socket, session reuse, autohide', flush=True)
        except Exception:
            log.flush()
            print(log.name + ':\n' + pathlib.Path(log.name).read_text(), file=sys.stderr)
            print('key records:', records(), file=sys.stderr)
            raise
        finally:
            for process in reversed(processes):
                process.terminate()
            for process in reversed(processes):
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
            log.close()
            keyboard.close()


if __name__ == '__main__':
    main()
