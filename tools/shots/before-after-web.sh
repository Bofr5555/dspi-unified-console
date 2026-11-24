#!/bin/bash
# -- WP SHOW2 -- the web console's before/after stills: a released web console (default 0.14.0's, the "before")
# and this tree's built page (the "after"), each serving its own files, the core's demo DSPi opened, the same
# screens taken (Main/Overview, EQ, Matrix/Routing, Effects, Measure, Snapshots, Events, ...) dark and light at
# 1366x900, into OUT/old and OUT/new. No network, no device: Playwright's image, the page's own demo.
#
#   tools/shots/before-after-web.sh OUT [OLD_DIST] [NEW_DIST]
#
# OLD_DIST defaults to the 0.14.0 web release (work/rel-0.14.0b-web/...), NEW_DIST to work/web/<branch>/dist
# (apps/web/tools/ci.sh --no-e2e builds it; SKIP_BUILD=1 takes it as it is). Playwright comes from that
# work folder's node_modules.
set -euo pipefail
out=${1:?usage: before-after-web.sh OUT [OLD_DIST] [NEW_DIST]}
root=$(cd "$(dirname "$0")/../.." && pwd)
. "$root/ci/container/images.conf"
B=${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}
branch=$(git -C "$root" rev-parse --abbrev-ref HEAD 2>/dev/null || echo detached)
# -- WP SHOWFIX -- a detached worktree is named by its short commit, not "HEAD"
if [ "$branch" = HEAD ]; then branch=$(git -C "$root" rev-parse --short HEAD); fi
work=$B/work/web/${branch//\//-}
old=${2:-$B/work/rel-0.14.0b-web/uConsole-for-DSPi-0.14.0-web}
new=${3:-$work/dist}
modules=${NODE_MODULES:-$work/node_modules}
[ -d "$modules" ] || { echo "before-after-web: no Playwright in $modules (set NODE_MODULES, e.g. work/web/main/node_modules)" >&2; exit 1; }
[ "${SKIP_BUILD:-}" = 1 ] || [ -n "${3:-}" ] || "$root/apps/web/tools/ci.sh" --no-e2e
mkdir -p "$out/old" "$out/new" "$B/work/empty"
out=$(cd "$out" && pwd)
for side in old new; do
  dist=$old; [ $side = new ] && dist=$new
  podman run --rm --init --userns=keep-id --user "$(id -u):$(id -g)" --network none --ipc=private --shm-size=1g \
    -v "$root/tools/shots/before-after-web.mjs":/shots/before-after-web.mjs:ro,z -v "$modules":/shots/node_modules:ro,z \
    -v "$dist":/dist:ro,z -v "$out/$side":/out:z -v "$B/work/empty":/sys/bus/usb/devices:ro,z \
    -e HOME=/tmp -w /shots "$PLAYWRIGHT" node /shots/before-after-web.mjs /dist /out $side
done
echo "before-after-web: $(ls "$out/old" | wc -l) before, $(ls "$out/new" | wc -l) after in $out"
