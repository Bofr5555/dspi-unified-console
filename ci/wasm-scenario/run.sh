#!/bin/bash
# The wasm gate's run (WP WB2): build the scenario module for
# wasm32-unknown-unknown in the cross container, then run it under Node with
# JSPI (NODE in ci/container/images.conf, pinned by digest, no network): two
# demo sessions, connect, a write with read-back, notifications both ways, RTA
# frames on caller-driven ticks, and an AutoEQ rebuild into bytes. The core
# runs with no threads, as in a page's Worker. Called by
# `ci/check.sh --step wasm-core`.
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
. "$root/ci/container/images.conf"
B=${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}
branch=$(git -C "$root" rev-parse --abbrev-ref HEAD 2>/dev/null || echo detached)
out=$B/target/${branch//\//-}-cross/wasm32-unknown-unknown/release
DSPI_PROFILE=cross "$root/ci/run.sh" \
  'cargo build -p dspi-wasm-scenario --release --target wasm32-unknown-unknown --locked'
wasm=$out/dspi_wasm_scenario.wasm
[ -s "$wasm" ] || { echo "wasm-scenario: no $wasm" >&2; exit 1; }
echo "wasm-scenario: $(stat -c %s "$wasm") bytes, $(gzip -9c "$wasm" | wc -c) gzip"
podman image exists "$NODE" || podman pull "$NODE"
# Node 24 has JSPI behind this flag; run.mjs refuses to start without it.
podman run --rm -i --network=none --userns=keep-id --user "$(id -u):$(id -g)" \
  -v "$root/ci/wasm-scenario":/scenario:ro,z -v "$out":/wasm:ro,z \
  "$NODE" node --experimental-wasm-jspi /scenario/run.mjs /wasm/dspi_wasm_scenario.wasm
