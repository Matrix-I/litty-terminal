#!/usr/bin/env python3
"""Native Wayland window smoke check under a headless Weston.

Opens Litty with `LITTY_BACKEND=wayland` on `weston --backend=headless
--shell=kiosk` and reads the client's protocol log (`WAYLAND_DEBUG`):

- open: an `xdg_toplevel` with app id `litty`, configured and drawn into;
- resize: the kiosk shell fullscreens every toplevel, so the window is
  configured from its own 968x620 to the output's size and must draw a
  buffer of that size;
- frame pacing: frames wait for the compositor's frame callbacks;
- close: when the shell exits, the toplevel is destroyed and Litty exits 0.

A close requested by the compositor (Alt+F4) cannot be sent by headless
Weston and is checked by hand. Input is not part of the backend yet.
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
SOCKET = 'litty-smoke'


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
    if shutil.which('weston') is None:
        print('SKIP Wayland smoke: weston is not installed', flush=True)
        return
    with tempfile.TemporaryDirectory(prefix='litty-wayland-') as temporary:
        root = pathlib.Path(temporary)
        home = root / 'home'
        (home / '.litty').mkdir(parents=True)
        (home / '.litty/config').write_text(
            'auto-update = off\nshell-integration = none\nconfirm-close-surface = false\n')
        runtime = root / 'runtime'
        runtime.mkdir(mode=0o700)
        ready, quit_flag = root / 'ready', root / 'quit'
        shell = root / 'probe-shell'
        # Exits on request, which closes its tab and so the only window.
        shell.write_text(f'''#!/bin/sh
: > '{ready}'
while [ ! -e '{quit_flag}' ]; do sleep 0.1; done
''')
        shell.chmod(0o700)
        env = dict(os.environ, HOME=str(home), SHELL=str(shell), XDG_RUNTIME_DIR=str(runtime))
        env.pop('DISPLAY', None)
        compositor_log = (root / 'weston.log').open('w+')
        client_log = (root / 'litty.log').open('w+')
        processes = []
        try:
            weston = subprocess.Popen(
                ['weston', '--backend=headless', '--renderer=pixman', '--shell=kiosk', f'--socket={SOCKET}',
                 f'--width={OUTPUT[0]}', f'--height={OUTPUT[1]}', '--idle-time=0'],
                stdout=compositor_log, stderr=compositor_log, env=env)
            processes.append(weston)
            wait_for(lambda: (runtime / SOCKET).exists(), bool, 'weston socket')

            app_env = dict(env, WAYLAND_DISPLAY=SOCKET, LITTY_BACKEND='wayland', WAYLAND_DEBUG='client')
            app = subprocess.Popen([binary], stdout=client_log, stderr=client_log, env=app_env)
            processes.append(app)

            def protocol():
                client_log.flush()
                return pathlib.Path(client_log.name).read_text(errors='replace')

            wait_for(ready.exists, bool, 'shell start')
            log = wait_for(protocol, lambda t: re.search(r'xdg_toplevel#\d+\.set_app_id\("litty"\)', t), 'app id')
            assert 'opening an X11 window instead' not in log, log[-2000:]
            print('PASS Wayland toplevel opens with app id litty', flush=True)

            width, height = OUTPUT
            wait_for(protocol, lambda t: re.search(rf'xdg_toplevel#\d+\.configure\({width}, {height},', t),
                     'fullscreen configure')
            # Software Vulkan presents through wl_shm buffers, which name their size.
            wait_for(protocol, lambda t: re.search(rf'create_buffer\(new id wl_buffer#\d+, \d+, {width}, {height},', t),
                     f'a {width}x{height} buffer after the resize')
            print(f'PASS Wayland window resizes to the configured {width}x{height}', flush=True)

            wait_for(protocol, lambda t: len(re.findall(r'wl_callback#\d+\.done', t)) >= 2, 'frame callbacks')
            print('PASS Wayland frames follow frame callbacks', flush=True)

            quit_flag.touch()
            code = app.wait(timeout=20)
            log = protocol()
            assert code == 0, (code, log[-2000:])
            assert re.search(r'-> xdg_toplevel#\d+\.destroy\(\)', log), log[-2000:]
            assert 'wl_display#1.error' not in log, log[-2000:]
            print('PASS Wayland window closes and Litty exits', flush=True)
        except Exception:
            for f in (compositor_log, client_log):
                f.flush()
                print(f'{f.name} (tail):\n' + pathlib.Path(f.name).read_text(errors='replace')[-6000:], file=sys.stderr)
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
