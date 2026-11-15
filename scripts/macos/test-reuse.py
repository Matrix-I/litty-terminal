#!/usr/bin/env python3
"""Real macOS GUI/PTY reuse checks; no synthetic UI or user dotfile changes.

Every launched process uses a private activation registry
(LITTY_ACTIVATION_DIR) and a temporary HOME, so the run never reaches a
person's running Litty, reads their config, or touches their update cache.
"""
import argparse
import os
from pathlib import Path
import select
import signal
import stat
import subprocess
import sys
import tempfile
import time


def require_empty_registry(registry):
    try:
        metadata = registry.lstat()
    except FileNotFoundError:
        return
    if not stat.S_ISDIR(metadata.st_mode) or metadata.st_uid != os.geteuid() or stat.S_IMODE(metadata.st_mode) != 0o700:
        raise RuntimeError("unsafe activation registry; refusing acceptance test")
    if next(registry.iterdir(), None) is not None:
        raise RuntimeError("activation registry is not empty; refuse to target any existing user instance")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", type=Path)
    parser.add_argument("--hold", action="store_true", help="keep the zero-window instance for a manual idle probe, at most 45 seconds")
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    processes = []
    # /tmp keeps endpoint socket paths below the Unix socket path limit.
    with tempfile.TemporaryDirectory(prefix="la-e2e-", dir="/tmp") as private, \
            tempfile.TemporaryDirectory(prefix="litty-reuse-e2e-", dir="/tmp") as temporary:
        registry = Path(private)
        # mkdtemp creates 0700 and owned by us; still refuse anything unexpected.
        require_empty_registry(registry)
        root = Path(temporary)
        home = root / "home"
        (home / ".litty").mkdir(parents=True)
        # No network update check or "update available" notification from the harness.
        (home / ".litty" / "config").write_text("auto-update = off\n")
        fifo = root / "shells"
        os.mkfifo(fifo, 0o600)
        pipe = os.open(fifo, os.O_RDWR | os.O_NONBLOCK)
        pending = bytearray()
        owned_endpoints = set()
        shell = root / "activation-shell"
        # TemporaryDirectory produces a trusted path; argv is never terminal input.
        shell.write_text(
            "#!/bin/sh\n"
            "actual=$(/bin/pwd -P) || exit 1\n"
            "case \"$actual\" in\n"
            "  /tmp/litty-reuse-e2e-*/*|/private/tmp/litty-reuse-e2e-*/*) label=${actual##*/} ;;\n"
            "  *) label=default ;;\n"
            "esac\n"
            f"printf '%s:%s\\n' \"$PPID\" \"$label\" > '{fifo}'\n"
            "exit 0\n"
        )
        shell.chmod(0o700)
        environment = dict(os.environ, SHELL=str(shell), HOME=str(home),
            LITTY_ACTIVATION_DIR=str(registry))
        for label in ("initial", "first tiếng việt", "newest", "after-crash"):
            (root / label).mkdir()

        def message():
            deadline = time.monotonic() + 10
            while b"\n" not in pending:
                left = deadline - time.monotonic()
                if left <= 0 or not select.select([pipe], [], [], left)[0]:
                    raise AssertionError("no PTY shell marker before deadline")
                pending.extend(os.read(pipe, 4096))
                if len(pending) > 16384:
                    raise AssertionError("excess shell markers")
            line, _, rest = pending.partition(b"\n")
            pending[:] = rest
            pid, label = line.decode().split(":", 1)
            return int(pid), label

        def wait_endpoint(process):
            # Vnode notifications avoid timing sleeps and wait for real readiness.
            deadline = time.monotonic() + 10
            queue = select.kqueue()
            watched = {}
            try:
                while True:
                    candidates = [registry]
                    for entry in registry.iterdir():
                        pieces = entry.name.split("-")
                        if len(pieces) == 3 and pieces[1] == f"{process.pid:x}":
                            candidates.append(entry)
                    for directory in candidates:
                        if directory not in watched:
                            fd = os.open(directory, os.O_RDONLY)
                            watched[directory] = fd
                            queue.control([select.kevent(fd, filter=select.KQ_FILTER_VNODE,
                                flags=select.KQ_EV_ADD | select.KQ_EV_CLEAR,
                                fflags=select.KQ_NOTE_WRITE)], 0)
                    for directory in candidates[1:]:
                        if (directory / "s").is_socket():
                            owned_endpoints.add(directory)
                            return
                    if process.poll() is not None:
                        raise AssertionError("GUI exited before service readiness")
                    left = deadline - time.monotonic()
                    if left <= 0 or not queue.control(None, 4, left):
                        raise AssertionError("activation endpoint not ready")
            finally:
                for fd in watched.values():
                    os.close(fd)
                queue.close()

        def client(label):
            return subprocess.run([str(binary), "--reuse"], cwd=root / label,
                env=environment, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
                text=True, timeout=6)

        try:
            first = subprocess.Popen([str(binary), "--reuse"], cwd=root / "initial",
                env=environment, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            processes.append(first)
            observed = message()
            assert observed == (first.pid, "initial"), f"no-instance reuse must start in caller cwd: {observed}, expected pid {first.pid}"
            wait_endpoint(first)
            assert client("first tiếng việt").returncode == 0
            assert message() == (first.pid, "first tiếng việt"), "Unicode cwd must reach real PTY child"
            second = subprocess.Popen([str(binary)], cwd=root / "newest", env=environment,
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            processes.append(second)
            # An ordinary launch starts in HOME (here the temporary home), not the caller cwd.
            observed = message()
            assert observed == (second.pid, "home"), f"ordinary launch must stay independent: {observed}"
            wait_endpoint(second)
            assert client("newest").returncode == 0
            assert message() == (second.pid, "newest"), "newest eligible process must win"
            os.kill(second.pid, signal.SIGSTOP)
            failed = client("newest")
            assert failed.returncode != 0, "paused delivery must time out"
            assert not pending and not select.select([pipe], [], [], 0)[0], "timeout must not fall back to older process"
            second.kill()
            second.wait(timeout=5)
            assert client("after-crash").returncode == 0
            assert message() == (first.pid, "after-crash"), "verified crash endpoint must not block remaining instance"
            assert first.poll() is None, "zero-window macOS instance must stay available"
            print("PASS: no-instance startup, Unicode PTY cwd, ordinary independent launch, newest reuse, timeout without fallback, crash recovery, zero-window reuse", flush=True)
            if args.hold:
                print(f"HOLD: zero-window test PID {first.pid}; press Enter to clean up", flush=True)
                if select.select([sys.stdin], [], [], 45)[0]:
                    sys.stdin.readline()
        finally:
            # Each step is independent so one failure neither skips the rest
            # nor hides the original assertion.
            errors = []
            for process in processes:
                try:
                    if process.poll() is None:
                        process.kill()
                    process.wait(timeout=5)
                except Exception as error:  # noqa: BLE001 - reported below
                    errors.append(f"stop {process.pid}: {error}")
            os.close(pipe)
            # Killed children skip their own cleanup; their endpoints may not have
            # been recorded if an assertion fired before readiness.
            pids = {f"{process.pid:x}" for process in processes}
            for entry in list(registry.iterdir()):
                pieces = entry.name.split("-")
                if entry in owned_endpoints or (len(pieces) == 3 and pieces[1] in pids):
                    try:
                        (entry / "s").unlink(missing_ok=True)
                        entry.rmdir()
                    except FileNotFoundError:
                        pass
                    except OSError as error:
                        errors.append(f"remove {entry.name}: {error}")
            if errors:
                print("CLEANUP: " + "; ".join(errors), file=sys.stderr, flush=True)


if __name__ == "__main__":
    main()
