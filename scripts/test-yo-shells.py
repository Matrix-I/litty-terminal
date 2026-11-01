#!/usr/bin/env python3
"""Isolated PTY smoke tests for yo interception on macOS zsh and Bash 3.2.
Run outside a PTY-restricted sandbox; no API keys or network requests are used.
"""
import os, pty, select, time, signal, tempfile, pathlib, fcntl, termios, struct, base64, shutil, shlex
ROOT=pathlib.Path(__file__).resolve().parents[1]
def read(fd, seconds=.4):
    end=time.monotonic()+seconds; data=b''
    while time.monotonic()<end:
        ready,_,_=select.select([fd],[],[],max(0,end-time.monotonic()))
        if not ready: break
        try: data+=os.read(fd,65536)
        except OSError: break
    return data
for shell in ['zsh','bash']:
    binary=shutil.which(shell)
    if not binary:
        print(shell+': skipped (not installed)')
        continue
    with tempfile.TemporaryDirectory(prefix='litty-yo-') as home:
        if shell=='zsh':
            pathlib.Path(home,'.zshrc').write_text(f"PROMPT='test> '\nsource {shlex.quote(str(ROOT / 'src/shell/litty-integration.zsh'))}\n")
            args=[binary,'-i']
        else:
            args=[binary,'--noprofile','--rcfile',str(ROOT/'src/shell/litty.bash'),'-i']
        pid,fd=pty.fork()
        if pid==0:
            os.chdir(home)
            os.environ.update(HOME=home,ZDOTDIR=home,TERM='xterm-256color',PS1='test> ',HISTFILE='/dev/null')
            os.execv(args[0],args)
        try:
            fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack('HHHH',24,120,0,0))
            initial=read(fd)
            assert b'778;init' in initial,(shell,initial)
            query='find files > 100MB; echo SHOULD_NOT_EXECUTE'
            if b'778;init;direct' in initial:
                os.write(fd,('yo '+query).encode()); read(fd)
                # Simulate Litty's direct interception before it forwards Enter.
                os.write(fd,b'\x05\x15\r'); output=read(fd)
                assert b'SHOULD_NOT_EXECUTE\r\n' not in output,(shell,output)
            else:
                os.write(fd,('yo '+query+'\r').encode()); output=read(fd)
            if b'778;init;direct' not in initial:
                marker=b'\x1b]778;yo;'; assert marker in output,(shell,output)
                payload=output.split(marker,1)[1].split(b'\x07',1)[0]
                assert base64.b64decode(payload).decode().strip()==query,(shell,output)
            assert b'command not found' not in output,(shell,output)
            # The private repaint chord must emit prompt-ready, not insert literal escape text.
            os.write(fd,b'\x1b[24;8~'); repaint=read(fd)
            assert b'133;B' in repaint,(shell,repaint)
            # Prefill alone cannot execute. An explicit Enter executes after edits.
            os.write(fd,b'\x1b[200~printf AI_TEST\x1b[201~' if b'?2004h' in initial else b'printf AI_TEST'); prefill=read(fd)
            assert b'133;C' not in prefill,(shell,prefill)
            os.write(fd,b'_EDITED\r'); executed=read(fd)
            assert b'AI_TEST_EDITED' in executed and b'133;C' in executed,(shell,executed)
            os.write(fd,b'printf NORMAL_TEST\r'); normal=read(fd)
            assert b'NORMAL_TEST' in normal and b'133;C' in normal,(shell,normal)
            print(shell+': bridge punctuation, repaint, editable nonexecuting prefill, normal Enter passed')
        finally:
            os.kill(pid,signal.SIGKILL);os.close(fd);os.waitpid(pid,0)
