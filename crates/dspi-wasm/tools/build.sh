#!/bin/bash
# WP WB3: build the web binding for wasm32-unknown-unknown in the cross
# container (DSPI_PROFILE=cross ci/run.sh), twice:
#
#   dspi_wasm.wasm        the module the page's Worker loads (apps/web/dist/)
#   dspi_wasm_test.wasm   the same with `test-hooks` (a call that suspends, a
#                         call that panics), for the binding's own tests only
#
#   crates/dspi-wasm/tools/build.sh <out-dir>
#
# Profile `wasm-web`: release with opt-level "s" (size over speed; the core is
# transfer-bound), set on the command line so the workspace's release
# profile and the wasm-core step's cache are untouched. Prints the sizes.
set -euo pipefail
root=$(cd "$(dirname "$0")/../../.." && pwd)
out=${1:?usage: build.sh <out-dir>}
mkdir -p "$out"
B=${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}
branch=$(git -C "$root" rev-parse --abbrev-ref HEAD 2>/dev/null || echo detached)
built=$B/target/${DSPI_TARGET_KEY:-${branch//\//-}}-cross/wasm32-unknown-unknown/wasm-web
profile="--config profile.wasm-web.inherits=\\\"release\\\" --config profile.wasm-web.opt-level=\\\"s\\\" --profile wasm-web"
DSPI_PROFILE=cross "$root/ci/run.sh" "set -e
  cargo build -p dspi-wasm $profile --target wasm32-unknown-unknown --locked
  cp /target/wasm32-unknown-unknown/wasm-web/dspi_wasm.wasm /target/wasm32-unknown-unknown/wasm-web/dspi_wasm.page.wasm
  cargo build -p dspi-wasm --features test-hooks $profile --target wasm32-unknown-unknown --locked
  cp /target/wasm32-unknown-unknown/wasm-web/dspi_wasm.wasm /target/wasm32-unknown-unknown/wasm-web/dspi_wasm.test.wasm"
cp "$built/dspi_wasm.page.wasm" "$out/dspi_wasm.wasm"
cp "$built/dspi_wasm.test.wasm" "$out/dspi_wasm_test.wasm"
for f in dspi_wasm.wasm dspi_wasm_test.wasm; do
  echo "dspi-wasm: $f $(stat -c %s "$out/$f") bytes, $(gzip -9c "$out/$f" | wc -c) gzip"
done
