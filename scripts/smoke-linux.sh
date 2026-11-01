#!/usr/bin/env bash
# Launch a private, empty workspace on a virtual X11 display; never touch live sessions.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
[[ "$ghostlink_os" == linux ]] || ghostlink_fail 'This smoke test uses Linux Xvfb.'
[[ $# -le 1 ]] || ghostlink_fail 'Usage: bash scripts/smoke-linux.sh [path-to-ghostlink]'
for tool in xvfb-run xwininfo; do
    command -v "$tool" >/dev/null || ghostlink_fail "Install $tool to run the Linux startup smoke test."
done
binary="${1:-$CARGO_TARGET_DIR/release/ghostlink}"
[[ -x "$binary" ]] || ghostlink_fail "Build/package the app first: $binary"
binary="$(realpath "$binary")"
temporary_root="${TMPDIR:-/tmp}"
run_dir="$(mktemp -d "$temporary_root/ghostlink-smoke.XXXXXX")"
cleanup() {
    case "$run_dir" in "$temporary_root/ghostlink-smoke."*) rm -rf -- "$run_dir" ;; esac
}
trap cleanup EXIT
mkdir -p "$run_dir/data" "$run_dir/config"
export XDG_DATA_HOME="$run_dir/data" XDG_CONFIG_HOME="$run_dir/config"
export WAYLAND_DISPLAY='' XDG_SESSION_TYPE=x11
xvfb-run -a -s '-screen 0 1280x820x24' bash -s -- "$binary" "$run_dir" <<'SMOKE'
set -euo pipefail
binary="$1"
run_dir="$2"
cd "$run_dir"
"$binary" </dev/null >"$run_dir/startup.log" 2>&1 &
child=$!
trap 'kill "$child" 2>/dev/null || true; wait "$child" 2>/dev/null || true' EXIT
found=false
for ((attempt=0; attempt<100; attempt++)); do
    if ! kill -0 "$child" 2>/dev/null; then
        cat "$run_dir/startup.log" >&2
        echo 'Ghostlink exited during Linux startup.' >&2
        exit 1
    fi
    if xwininfo -root -tree | grep -F -q '"Ghostlink"'; then
        found=true
        break
    fi
    sleep 0.1
done
if [[ "$found" != true ]]; then
    cat "$run_dir/startup.log" >&2
    echo 'Ghostlink did not create its window.' >&2
    exit 1
fi
sleep 2
if ! kill -0 "$child" 2>/dev/null; then
    cat "$run_dir/startup.log" >&2
    exit 1
fi
echo 'Linux startup smoke test passed: window created and process remained alive.'
SMOKE
