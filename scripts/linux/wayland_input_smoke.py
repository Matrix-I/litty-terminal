#!/usr/bin/env python3
"""Native Wayland input and scale smoke check under a headless sway.

Weston's headless backend has no seat, so input runs on sway (wlroots),
which accepts virtual keyboards (`wtype`) and pointers (`wlrctl`). Litty
opens with `LITTY_BACKEND=wayland` and a probe shell that records the lines
it reads from the terminal:

- typing: text with Shift and punctuation arrives as typed, through the
  keymap the virtual keyboard sends;
- modifiers: Ctrl+W reaches the terminal as the word-erase character;
- key repeat: a held key repeats on the client and stops on release;
- pointer: a new pointer is bound with a `wp_cursor_shape_v1` device and
  released when it goes away (the cursor on enter is checked by hand);
- scale: at 150% the window gets the fractional scale, keeps its logical
  size through a viewport and draws a buffer 1.5 times that size.
"""
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile
import time

OUTPUT = (1024, 640)
TOOLS = ('sway', 'swaymsg', 'wtype', 'wlrctl')
# 150%, in the 120ths `wp_fractional_scale_v1` speaks.
SCALE_120 = 180
# wtype's pause before its first key, for Litty to bind the new keyboard.
TYPING_PAUSE_MS = 2000


def wait_for(read, condition, label, seconds=20):
    deadline = time.monotonic() + seconds
    last = None
    while time.monotonic() < deadline:
        last = read()
        if condition(last):
            return last
        time.sleep(.1)
    raise AssertionError(f'{label}: {last!r}'[-2000:])


def main():
    binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else 'target/debug/litty').resolve())
    missing = [t for t in TOOLS if shutil.which(t) is None]
    if missing:
        print(f'SKIP Wayland input smoke: {", ".join(missing)} not installed', flush=True)
        return
    with tempfile.TemporaryDirectory(prefix='litty-wayland-sway-') as temporary:
        root = pathlib.Path(temporary)
        home = root / 'home'
        (home / '.litty').mkdir(parents=True)
        (home / '.litty/config').write_text(
            'auto-update = off\nshell-integration = none\nconfirm-close-surface = false\n')
        runtime = root / 'runtime'
        runtime.mkdir(mode=0o700)
        ready, quit_flag, typed = root / 'ready', root / 'quit', root / 'typed'
        shell = root / 'probe-shell'
        # Records three lines from the terminal, then waits to be told to exit.
        shell.write_text(f'''#!/bin/sh
: > '{ready}'
for _ in 1 2 3; do IFS= read -r line; printf '%s\\n' "$line" >> '{typed}'; done
while [ ! -e '{quit_flag}' ]; do sleep 0.1; done
''')
        shell.chmod(0o700)
        config = root / 'sway.conf'
        config.write_text(f'output HEADLESS-1 resolution {OUTPUT[0]}x{OUTPUT[1]}\ndefault_border none\n')
        env = dict(os.environ, HOME=str(home), SHELL=str(shell), XDG_RUNTIME_DIR=str(runtime),
                   WLR_BACKENDS='headless', WLR_RENDERER='pixman', WLR_LIBINPUT_NO_DEVICES='1')
        for name in ('DISPLAY', 'WAYLAND_DISPLAY', 'SWAYSOCK'):
            env.pop(name, None)
        compositor_log = (root / 'sway.log').open('w+')
        client_log = (root / 'litty.log').open('w+')
        processes = []
        try:
            sway = subprocess.Popen(['sway', '-c', str(config)], stdout=compositor_log, stderr=compositor_log, env=env)
            processes.append(sway)
            socket = wait_for(lambda: next((p.name for p in runtime.glob('wayland-*') if not p.name.endswith('.lock')), None),
                              bool, 'sway socket')

            app_env = dict(env, WAYLAND_DISPLAY=socket, LITTY_BACKEND='wayland', WAYLAND_DEBUG='client')
            app = subprocess.Popen([binary], stdout=client_log, stderr=client_log, env=app_env)
            processes.append(app)

            def protocol():
                client_log.flush()
                return pathlib.Path(client_log.name).read_text(errors='replace')

            def lines():
                return typed.read_text().splitlines() if typed.exists() else []

            def tool(*args):
                subprocess.run(args, env=dict(env, WAYLAND_DISPLAY=socket), check=True, timeout=20)

            def typing(*args):
                # Headless sway has no keyboard until wtype's virtual one
                # appears, and keys sent before a client binds it never reach
                # that client. wtype pauses first; both of Litty's keyboards
                # (the window's and the clipboard's) must enter within it.
                entered = len(re.findall(r'wl_keyboard#\d+\.enter\(', protocol()))
                typist = subprocess.Popen(['wtype', '-s', str(TYPING_PAUSE_MS), *args],
                                          env=dict(env, WAYLAND_DISPLAY=socket))
                try:
                    wait_for(protocol, lambda t: len(re.findall(r'wl_keyboard#\d+\.enter\(', t)) >= entered + 2,
                             'keyboard bound before typing', seconds=TYPING_PAUSE_MS / 1000 - .3)
                    assert typist.wait(timeout=20) == 0
                finally:
                    if typist.poll() is None:
                        typist.kill()
                        typist.wait()

            wait_for(ready.exists, bool, 'shell start')
            wait_for(protocol, lambda t: re.search(r'xdg_toplevel#\d+\.configure\(', t), 'window shown')
            assert 'opening an X11 window instead' not in protocol()

            text = 'Hello, Wayland! ~/x_1'
            typing(text, '-k', 'Return')
            wait_for(lines, lambda l: len(l) >= 1, 'typed line')
            assert lines()[0] == text, lines()
            print('PASS Wayland keys type shifted text through the keymap', flush=True)

            # The tty erases the word on ^W: Ctrl reached the terminal. (Ctrl+U
            # is no probe: at a prompt Litty turns it into DELs counted from the
            # echoed line, which a burst of keys can outrun.)
            typing('junk', '-M', 'ctrl', 'w', '-m', 'ctrl', 'ok', '-k', 'Return')
            wait_for(lines, lambda l: len(l) >= 2, 'line after Ctrl+W')
            assert lines()[1] == 'ok', lines()
            print('PASS Wayland Ctrl+W reaches the terminal as a control key', flush=True)

            typing('-P', 'x', '-s', '1500', '-p', 'x', '-s', '700', '-k', 'Return')
            wait_for(lines, lambda l: len(l) >= 3, 'repeated line')
            repeated = lines()[2]
            # Sway repeats 25/s after 600 ms: 1 + about 23 in 1.5 s; a repeat
            # left running past the release would add 17 more in the pause.
            assert set(repeated) == {'x'} and 5 <= len(repeated) <= 35, repeated
            print(f'PASS Wayland held key repeats on the client ({len(repeated)}x) and stops on release', flush=True)

            # wlrctl's virtual pointer lives for one motion, gone before Litty's
            # pointer gets an enter, so the cursor shown on enter is checked
            # on a real compositor; here the pointer must be bound with a
            # cursor-shape device, and released with it when it goes away.
            tool('wlrctl', 'pointer', 'move', '200', '200')
            wait_for(protocol, lambda t: re.search(r'-> wp_cursor_shape_manager_v1#\d+\.get_pointer\(', t), 'pointer bound')
            wait_for(protocol, lambda t: re.search(r'-> wp_cursor_shape_device_v1#\d+\.destroy\(\)', t), 'pointer released')
            print('PASS Wayland pointer is bound with a cursor shape device and released with it', flush=True)

            ipc = wait_for(lambda: next(runtime.glob('sway-ipc.*.sock'), None), bool, 'sway IPC socket')
            subprocess.run(['swaymsg', '-s', str(ipc), 'output', 'HEADLESS-1', 'scale', str(SCALE_120 / 120)],
                           env=env, check=True, timeout=20, stdout=subprocess.DEVNULL)
            preferred = rf'wp_fractional_scale_v1#\d+\.preferred_scale\({SCALE_120}\)'
            scaled_at = re.search(preferred, wait_for(protocol, lambda t: re.search(preferred, t), 'fractional scale')).end()
            destination = wait_for(lambda: re.search(r'-> wp_viewport#\d+\.set_destination\((\d+), (\d+)\)', protocol()[scaled_at:]),
                                   bool, 'viewport at the new logical size')
            logical = tuple(int(v) for v in destination.groups())
            # The viewport shows the size sway configured, not a stale one.
            configured = re.findall(r'xdg_toplevel#\d+\.configure\((\d+), (\d+),', protocol()[scaled_at:])
            assert configured and tuple(map(int, configured[-1])) == logical, (configured, logical)
            # Rounded half away from zero, as the protocol asks.
            expected = tuple(int(v * SCALE_120 / 120 + .5) for v in logical)

            def last_buffer(text):
                sizes = re.findall(r'create_buffer\(new id wl_buffer#\d+, \d+, (\d+), (\d+),', text)
                return tuple(map(int, sizes[-1])) if sizes else None

            wait_for(lambda: last_buffer(protocol()[scaled_at:]), lambda size: size == expected, f'a {expected[0]}x{expected[1]} buffer')
            print(f'PASS Wayland window at 150% shows {logical[0]}x{logical[1]} with a {expected[0]}x{expected[1]} buffer', flush=True)

            quit_flag.touch()
            code = app.wait(timeout=20)
            log = protocol()
            assert code == 0, (code, log[-2000:])
            assert 'wl_display#1.error' not in log, log[-2000:]
            print('PASS Wayland input session closes cleanly', flush=True)
        except Exception:
            for f in (compositor_log, client_log):
                f.flush()
                print(f'{f.name} (tail):\n' + pathlib.Path(f.name).read_text(errors='replace')[-6000:], file=sys.stderr)
            if typed.exists():
                print(f'typed: {typed.read_text()!r}', file=sys.stderr)
            raise
        finally:
            for process in reversed(processes):
                if process.poll() is None:
                    process.terminate()
            for process in reversed(processes):
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
            compositor_log.close()
            client_log.close()


if __name__ == '__main__':
    main()
