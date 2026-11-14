#!/usr/bin/env python3
"""Live GNOME activation/occlusion test; requires an unlocked idle desktop.

Uses the GTK4/Mutter helpers from wayland_data_smoke.py. A GDK launcher
supplies an activation token to an isolated Litty with an always-animated
shader. Checks token removal before PTY spawn, drawable release and idle CPU
while covered, asynchronous bell attention, rendering recovery, and focus
requests from keyboard input. Leaves private artifacts in the printed /tmp
folder for inspection. Run explicitly with /usr/bin/python3.
"""
import sys
sys.dont_write_bytecode = True
from wayland_data_smoke import Input, pump, wait_for
from gi.repository import Gtk, Gdk, Gio, GLib
import pathlib, tempfile, os, subprocess, re, json, time, signal

def main():
    binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else 'target/debug/litty').resolve())
    control = Input()
    Gdk.set_allowed_backends('wayland')
    Gtk.init()
    peer = Gtk.Window(title='Litty activation and occlusion test')
    peer.set_default_size(450, 260)
    peer.set_child(Gtk.Label(label='Launcher and full-screen cover'))
    peer.present()
    pump(0.8)
    assert peer.is_active(), 'launcher focus'
    control.chord([], 42)
    root = pathlib.Path(tempfile.mkdtemp(prefix='litty-phase7-'))
    pid = None
    try:
        (root / '.litty').mkdir()
        shader = root / 'identity.glsl'
        shader.write_text('void mainImage(out vec4 color, in vec2 coord) { color = texture(iChannel0, coord / iResolution.xy); }\n')
        (root / '.litty/config').write_text('auto-update = off\nshell-integration = none\nconfirm-close-surface = false\ncursor-style-blink = false\ncustom-shader-animation = always\ncustom-shader = ' + str(shader) + '\n')
        shell = root / 'shell'
        shell.write_text(f"""#!/bin/bash\n    if [ -n "${{XDG_ACTIVATION_TOKEN+x}}" ]; then echo inherited; else echo absent; fi > '{root / 'shell-env'}'\n    printf 'Phase 7 probe\\n'\n    while [ ! -e '{root / 'quit'}' ]; do\n     if [ -e '{root / 'bell'}' ] && [ ! -e '{root / 'rang'}' ]; then printf '\\a'; touch '{root / 'rang'}'; fi\n     sleep 0.05\n    done\n    """)
        shell.chmod(448)
        helper = root / 'launch'
        helper.write_text(f"#!/usr/bin/python3\n    import os,subprocess,pathlib,json\n    root=pathlib.Path({str(root)!r})\n    (root/'launcher-env').write_text(json.dumps({{'token':bool(os.environ.get('XDG_ACTIVATION_TOKEN'))}}))\n    with (root/'protocol').open('w') as log:\n     p=subprocess.Popen([{binary!r}],stderr=log,stdout=subprocess.DEVNULL)\n     (root/'pid').write_text(str(p.pid))\n     code=p.wait()\n     (root/'exit').write_text(str(code))\n    ".replace('\n    ', '\n'))
        helper.chmod(448)
        context = Gdk.Display.get_default().get_app_launch_context()
        for name, value in {'HOME': str(root), 'SHELL': str(shell), 'LITTY_BACKEND': 'wayland', 'WAYLAND_DEBUG': '1'}.items():
            context.setenv(name, value)
        info = Gio.AppInfo.create_from_commandline(str(helper), 'Litty phase 7 probe', Gio.AppInfoCreateFlags.SUPPORTS_STARTUP_NOTIFICATION)
        info.launch([], context)
        wait_for(lambda: (root / 'pid').exists(), bool, 'app launch')
        pid = int((root / 'pid').read_text())

        def trace():
            return (root / 'protocol').read_text(errors='replace')

        def frames():
            return len(re.findall('wl_surface#\\d+\\.frame\\(', trace()))

        def destroyed():
            return len(re.findall('wl_buffer#\\d+\\.destroy\\(', trace()))
        print('launcher token received', json.loads((root / 'launcher-env').read_text())['token'], flush=True)
        wait_for(lambda: (root / 'shell-env').exists(), bool, 'shell environment')
        assert (root / 'shell-env').read_text().strip() == 'absent', 'activation token leaked to PTY'
        assert json.loads((root / 'launcher-env').read_text())['token'], 'GDK did not supply activation token'
        wait_for(trace, lambda t: re.search('xdg_activation_v1#\\d+\\.activate\\(', t), 'startup activate')
        wait_for(frames, lambda n: n > 20, 'visible animation')
        print('PASS launcher token activates and is absent in PTY environment', flush=True)
        before_destroy = destroyed()
        peer.fullscreen()
        peer.present()
        pump(0.5)
        if not peer.is_active():
            control.chord([56], 15)
        wait_for(peer.is_active, bool, 'cover focus')
        wait_for(destroyed, lambda n: n > before_destroy, 'starved frame releases drawables', seconds=8)
        pump(0.5)
        before_frames = frames()

        def ticks():
            fields = pathlib.Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
            return int(fields[11]) + int(fields[12])
        before_ticks = ticks()
        pump(3)
        used = ticks() - before_ticks
        hidden_frames = frames() - before_frames
        print('hidden frames', hidden_frames, 'CPU ticks / 3s', used, flush=True)
        assert hidden_frames <= 2, 'animation still submits while covered'
        assert used <= 3, 'idle timer keeps waking while covered'
        mark = len(trace())
        (root / 'bell').touch()
        wait_for(lambda: trace()[mark:], lambda t: re.search('xdg_activation_v1#\\d+\\.get_activation_token\\(', t), 'bell token request')
        wait_for(lambda: trace()[mark:], lambda t: re.search('xdg_activation_v1#\\d+\\.activate\\(', t), 'attention activation')
        print('PASS hidden event loop services asynchronous attention', flush=True)
        before_frames = frames()
        peer.set_visible(False)
        wait_for(frames, lambda n: n > before_frames + 10, 'frames resume after uncover', seconds=8)
        print('PASS occlusion stops animation and callbacks restore rendering', flush=True)
        mark = len(trace())
        focus = re.findall('zwp_text_input_v3#\\d+\\.(enter|leave)\\(', trace())
        assert focus and focus[-1] == 'enter', 'test window lost focus; refusing keyboard input'
        control.chord([29, 42], 49)
        wait_for(lambda: trace()[mark:], lambda t: re.search('xdg_activation_token_v1#\\d+\\.set_serial\\(', t), 'focus input serial')
        wait_for(lambda: trace()[mark:], lambda t: re.search('xdg_activation_token_v1#\\d+\\.set_surface\\(', t), 'focus source surface')
        wait_for(lambda: trace()[mark:], lambda t: re.search('xdg_activation_v1#\\d+\\.activate\\(', t), 'new window activation')
        print('PASS focus requests carry the recent seat/serial and source surface', flush=True)
        (root / 'quit').touch()
        wait_for(lambda: (root / 'exit').exists(), bool, 'clean exit')
        assert (root / 'exit').read_text() == '0'
        assert not re.search('wl_display#\\d+\\.error\\(', trace())
        print('PASS phase 7 native GNOME session', flush=True)
    finally:
        (root / 'quit').touch()
        pump(0.2)
        if pid and (not (root / 'exit').exists()):
            try:
                os.kill(pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
        peer.close()
        control.call('Stop')
        print('Artifacts:', root, flush=True)
if __name__ == '__main__':
    main()
