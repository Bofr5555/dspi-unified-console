#!/bin/bash
# WP WB5: the web client's checks, in the official Playwright image (PLAYWRIGHT in
# ci/container/images.conf, pinned by digest). `ci/check.sh --step web` runs this.
#
#   apps/web/tools/ci.sh            the core (crates/dspi-wasm, built in the cross
#                                   container), install (npm ci, from the lockfile),
#                                   typecheck, build, unit tests (the core's under
#                                   JSPI too), e2e in Chromium and Firefox; Link's
#                                   against dspi-link's fake hub (WB4b)
#   apps/web/tools/ci.sh --no-e2e   all but the browsers
#
# npm installs only what package-lock.json names, with its integrity hashes. The
# npm cache and node_modules live under ${DSPI_BUILD_CACHE:-~/.cache/dspi-build}/work/web
# (never tmpfs); the source tree is mounted read-only apart from apps/web's build
# outputs. Runs as the host user. The browsers are the image's own.
set -euo pipefail
root=$(cd "$(dirname "$0")/../../.." && pwd)
. "$root/ci/container/images.conf"
B=${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}
branch=$(git -C "$root" rev-parse --abbrev-ref HEAD 2>/dev/null || echo detached)
work=$B/work/web/${branch//\//-}
mkdir -p "$work/npm" "$work/node_modules" "$work/home" "$work/tmp" "$work/dist" "$work/dist-test" "$work/results" "$work/wasm"
e2e=1
[ "${1:-}" = --no-e2e ] && e2e=0

# -- WP WB3 -- the core for the page (dspi_wasm.wasm) and for its own tests
# (dspi_wasm_test.wasm), in the cross container; build.mjs copies the first
# into dist/, the unit tests load both.
"$root/crates/dspi-wasm/tools/build.sh" "$work/wasm"

# -- WP WB4b -- the fake hub the Link e2e tests reach through tools/hub-host.mjs,
# built in the cross container (Ubuntu 24.04, as the Playwright image) so it runs there
mkdir -p "$work/hub"
DSPI_PROFILE=cross "$root/ci/run.sh" "cargo build -p dspi-link --features fake-hub --bin dspi-link-fakehub --locked"
cp "$B/target/${branch//\//-}-cross/debug/dspi-link-fakehub" "$work/hub/"

podman image exists "$PLAYWRIGHT" || podman pull -q "$PLAYWRIGHT" >/dev/null
podman run --rm --init --userns=keep-id --user "$(id -u):$(id -g)" --ipc=private \
  -v "$root":/src:ro,z \
  -v "$work/node_modules":/src/apps/web/node_modules:z \
  -v "$work/dist":/src/apps/web/dist:z \
  -v "$work/dist-test":/src/apps/web/dist-test:z \
  -v "$work/results":/results:z -e PW_OUTPUT_DIR=/results/out \
  -v "$work/wasm":/wasm:ro,z -e DSPI_WASM_DIR=/wasm -e DSPI_WASM_REQUIRED=1 \
  -v "$work/hub":/hub:ro,z -e DSPI_FAKEHUB=/hub/dspi-link-fakehub -e DSPI_FAKEHUB_REQUIRED=1 \
  -v "$work/npm":/npm:z -v "$work/home":/home/web:z -v "$work/tmp":/wtmp:z \
  -e HOME=/home/web -e TMPDIR=/wtmp -e npm_config_cache=/npm -e npm_config_update_notifier=false \
  -e CI=1 -e E2E=$e2e -w /src/apps/web "$PLAYWRIGHT" bash -euo pipefail -c '
    npm ci --no-audit --no-fund --ignore-scripts
    node node_modules/esbuild/install.js
    npx tsc --version && npm run typecheck
    npm run test:unit
    node tools/build.mjs --pages
    ls -l dist
    if [ "$E2E" = 1 ]; then npx playwright test; fi'
echo "web: OK"
