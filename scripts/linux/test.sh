#!/usr/bin/env bash
# Build/run native Linux tests without exposing the host checkout or credentials.
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
runtime=${CONTAINER_RUNTIME:-}
if [[ -z "$runtime" ]]; then
    for candidate in podman docker; do
        if command -v "$candidate" >/dev/null && "$candidate" info >/dev/null 2>&1; then
            runtime=$candidate
            break
        fi
    done
fi
if [[ "$runtime" != podman && "$runtime" != docker ]]; then
    echo 'Start Podman/Docker, then rerun (or set CONTAINER_RUNTIME=podman|docker).' >&2
    exit 1
fi
context=$(mktemp -d)
trap 'rm -rf "$context"' EXIT
python3 - "$root" "$context" <<'PY'
import pathlib, shutil, subprocess, sys
root, dest = map(pathlib.Path, sys.argv[1:])
tracked = subprocess.check_output(['git', '-c', 'core.fsmonitor=false', '-C', str(root), 'ls-files', '-z']).decode().split('\0')
extra = [str(p.relative_to(root)) for p in (root/'scripts/linux').rglob('*') if p.is_file() and p.suffix != '.pyc']
for relative in set(tracked + extra):
    path = root / relative
    if not relative or not path.is_file(): continue
    target = dest / relative
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(path, target)
PY
build_args=(build -t localhost/litty-linux-test:local -f "$context/scripts/linux/Containerfile")
if [[ -n "${LITTY_TEST_CA_BUNDLE:-}" ]]; then
    build_args+=(--secret "id=host-ca,src=$LITTY_TEST_CA_BUNDLE")
fi
"$runtime" "${build_args[@]}" "$context"
"$runtime" run --rm --init --memory=3g --shm-size=256m --pids-limit=512 \
    -v litty-linux-target:/workspace/target \
    -v litty-linux-registry:/usr/local/cargo/registry \
    localhost/litty-linux-test:local
