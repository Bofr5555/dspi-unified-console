#!/bin/bash
# The Terminal's screenshot tour: `uconsole --demo` in a 132x40 pty (pty_shots.py, script
# terminal.json), each screen rendered to a dark PNG (render.mjs), in the Playwright image.
# Uses the `uconsole` and node_modules tools/shots/web.sh built (run it first).
#
#   tools/shots/terminal.sh <out dir>     <out dir>/html (screens) and <out dir>/png
set -euo pipefail
out=${1:?usage: terminal.sh <out dir>}
root=$(cd "$(dirname "$0")/../.." && pwd)
. "$root/ci/container/images.conf"
B=${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}
branch=$(git -C "$root" rev-parse --abbrev-ref HEAD 2>/dev/null || echo detached)
# -- WP SHOWFIX -- a detached worktree is named by its short commit, not "HEAD"
if [ "$branch" = HEAD ]; then branch=$(git -C "$root" rev-parse --short HEAD); fi
work=$B/work/web/${branch//\//-}
mkdir -p "$out/html" "$out/png" "$B/work/empty"
out=$(cd "$out" && pwd)
rm -rf "$work/tmp/shots-hub-store"
run=(podman run --rm --network=none --userns=keep-id --user "$(id -u):$(id -g)"
  -v "$B/work/empty":/sys/bus/usb/devices:ro,z -v "$root":/src:ro,z
  -v "$work/node_modules":/src/apps/web/node_modules:ro,z -v "$work/hub":/hub:ro,z
  -v "$work/tmp":/wtmp:z -v "$out":/out:z -e HOME=/wtmp/shots-home -e TMPDIR=/wtmp "$PLAYWRIGHT")
"${run[@]}" python3 /src/tools/shots/pty_shots.py /out/html /src/tools/shots/terminal.json
"${run[@]}" node /src/tools/shots/render.mjs /out/html /out/png terminal- dark
