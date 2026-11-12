#!/bin/bash
# WP WB7: the web console as a build artefact, nothing published (publishing is
# the owner's act). The core in the cross container (crates/dspi-wasm/tools/build.sh),
# the page in the pinned Playwright image (Node only; no tests, no test pages):
#
#   DSPI_WEB_SOURCE_URL=https://.../source apps/web/tools/dist.sh <out-dir>
#
# <out-dir> gets the dist (what `uconsole hub --console`, `uconsole web` or any static
# host serves; apps/web/DEPLOY.md) and SHA256SUMS. A release build: it refuses to
# build without DSPI_WEB_SOURCE_URL, the GPL source link About shows.
set -euo pipefail
root=$(cd "$(dirname "$0")/../../.." && pwd)
out=${1:?usage: dist.sh <out-dir>}
[ -n "${DSPI_WEB_SOURCE_URL:-}" ] || { echo "dist.sh: set DSPI_WEB_SOURCE_URL (the source link, GPL)" >&2; exit 2; }
. "$root/ci/container/images.conf"
B=${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}
branch=$(git -C "$root" rev-parse --abbrev-ref HEAD 2>/dev/null || echo detached)
work=$B/work/web-dist/${branch//\//-}
mkdir -p "$out" "$work/npm" "$work/node_modules" "$work/home" "$work/tmp" "$work/wasm"
out=$(cd "$out" && pwd)
"$root/crates/dspi-wasm/tools/build.sh" "$work/wasm"
podman image exists "$PLAYWRIGHT" || podman pull -q "$PLAYWRIGHT" >/dev/null
podman run --rm --init --userns=keep-id --user "$(id -u):$(id -g)" \
  -v "$root":/src:ro,z \
  -v "$work/node_modules":/src/apps/web/node_modules:z \
  -v "$out":/src/apps/web/dist:z \
  -v "$work/wasm":/wasm:ro,z -e DSPI_WASM_DIR=/wasm \
  -v "$work/npm":/npm:z -v "$work/home":/home/web:z -v "$work/tmp":/wtmp:z \
  -e HOME=/home/web -e TMPDIR=/wtmp -e npm_config_cache=/npm -e npm_config_update_notifier=false \
  -e DSPI_WEB_RELEASE=1 -e DSPI_WEB_SOURCE_URL="$DSPI_WEB_SOURCE_URL" \
  -w /src/apps/web "$PLAYWRIGHT" bash -euo pipefail -c '
    npm ci --no-audit --no-fund --ignore-scripts
    node node_modules/esbuild/install.js
    node tools/build.mjs'
[ -f "$out/dspi_wasm.wasm" ] || { echo "dist.sh: no core module in the dist" >&2; exit 1; }
(cd "$out" && find . -type f ! -name SHA256SUMS -printf '%P\n' | sort | xargs sha256sum > SHA256SUMS)
echo "dist: $out ($(du -sh "$out" | cut -f1))"
ls -l "$out"
