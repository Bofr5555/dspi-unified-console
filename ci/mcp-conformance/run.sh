#!/bin/bash
# WP MCP6: dspi-mcp against the official MCP conformance suite, in both spec eras, and
# against the official TypeScript SDK's client (`ci/check.sh --step mcp-conformance`).
#
#   ci/mcp-conformance/run.sh
#
# 1. The server is crates/dspi-mcp/tests/wp_mcp6_serve.rs's ignored test: the real
#    loopback listener (features http and link, as the app has them) over the demo device, under the app's default
#    policy (ask about risky changes), with one paired client. Built by ci/run.sh and run
#    in a container from the same image, with no USB.
# 2. Node from the official image (NODE in ci/container/images.conf), in the server's
#    network namespace; nothing is published. The packages are package-lock.json's
#    (npm ci, integrity-checked, no install scripts), cached by the lock's hash.
# 3. @modelcontextprotocol/conformance runs each spec revision's frozen requirement set
#    (`--requirements 2025-11-25`, then `2026-07-28`) through auth-proxy.mjs, which adds
#    the client's token (the suite cannot send headers). baseline-<revision>.yml lists
#    what the suite requires that this server deliberately does not serve, each with
#    its reason; a scenario that fails outside it, or passes inside it, fails the step.
# 4. sdk-client.mjs: the TypeScript SDK's client connects as an agent does and walks the
#    tools, resources and prompts.
#
# Scratch and results: ${DSPI_BUILD_CACHE:-~/.cache/dspi-build}/work/mcp6-conformance/,
# never tmpfs. Exit status: 0 when all three passed.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
. "$root/ci/container/images.conf"
B=${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}
NODE_IMAGE=${NODE_IMAGE:-$NODE}
work=$B/work/mcp6-conformance
mkdir -p "$work"
rm -rf "$work/conf" "$work/results" && mkdir -p "$work/conf/tmp" "$work/results" "$work/node/home"
log=$work/run.log
: >"$log"
say() { echo "$*" | tee -a "$log"; }

# 1. The server's binary.
say "building the server (dspi-mcp, features http and link)"
bin=$("$root/ci/run.sh" 'cargo test -p dspi-mcp --features http,link --test wp_mcp6_serve --no-run 2>&1' |
  tee -a "$log" | sed -n 's/.*Executable tests\/wp_mcp6_serve.rs (\(.*\))$/\1/p' | tail -1)
[ -n "$bin" ] || { say "no test binary (see $log)"; exit 1; }
branch=$(git -C "$root" rev-parse --abbrev-ref HEAD 2>/dev/null || echo detached)
target=$B/target/${DSPI_TARGET_KEY:-${branch//\//-}}  # -- WP CIPAR --

name=mcp6-serve-$$
cleanup() {
  touch "$work/conf/stop" 2>/dev/null || true
  podman rm -f -t 5 "$name" >/dev/null 2>&1 || true
}
trap cleanup EXIT
podman run -d --name "$name" --userns=keep-id --user "$(id -u):$(id -g)" \
  -v "$target":/target:ro,z -v "$work/conf":/conf:z -v "$B/work/empty":/sys/bus/usb/devices:ro,z \
  -e MCP6_CONF_DIR=/conf -e TMPDIR=/conf/tmp -e MCP6_SERVE_SECS=900 \
  "$COREMERGE" "$bin" --ignored --exact wp_mcp6_serve_for_the_conformance_suite --nocapture >/dev/null
for _ in $(seq 300); do [ -s "$work/conf/server.json" ] && break; sleep 0.1; done
[ -s "$work/conf/server.json" ] || { podman logs "$name" >>"$log" 2>&1; say "the server did not start (see $log)"; exit 1; }
server_json=$(cat "$work/conf/server.json")
field() { sed -n "s/.*\"$1\":\\(\"\\([^\"]*\\)\"\\|\\([0-9]*\\)\\).*/\\2\\3/p" <<<"$server_json"; }
port=$(field port)
token=$(field token)
say "server up on port $port"

# 2. The packages, from the lock.
lock_sum=$(sha256sum "$here/package-lock.json" | cut -d' ' -f1)
if [ "$(cat "$work/node/.lock-sha256" 2>/dev/null)" != "$lock_sum" ]; then
  say "npm ci (package-lock.json changed or first run)"
  rm -rf "$work/node/node_modules"
  cp "$here/package.json" "$here/package-lock.json" "$work/node/"
  podman run --rm --userns=keep-id --user "$(id -u):$(id -g)" -v "$work/node":/node:z \
    -e HOME=/node/home -w /node "$NODE_IMAGE" \
    npm ci --ignore-scripts --no-audit --no-fund >>"$log" 2>&1 || { say "npm ci failed (see $log)"; exit 1; }
  echo "$lock_sum" >"$work/node/.lock-sha256"
fi

# 3 and 4, in the server's network namespace.
status=0
podman run --rm -i --network="container:$name" --userns=keep-id --user "$(id -u):$(id -g)" \
  -v "$work/node":/node:z -v "$here":/mcp6:ro,z -v "$work/results":/results:z \
  -e HOME=/node/home -e SERVER_PORT="$port" -e MCP_TOKEN="$token" -e PROXY_PORT=47999 -w /results \
  "$NODE_IMAGE" bash -s >>"$log" 2>&1 <<'SCRIPT' || status=$?
set -uo pipefail
export NODE_PATH=/node/node_modules
node /mcp6/auth-proxy.mjs &
proxy=$!
for _ in $(seq 50); do (echo >/dev/tcp/127.0.0.1/$PROXY_PORT) 2>/dev/null && break; sleep 0.1; done
rc=0
for rev in 2025-11-25 2026-07-28; do
  echo "== conformance --requirements $rev"
  baseline=()
  [ -s "/mcp6/baseline-$rev.yml" ] && baseline=(--expected-failures "/mcp6/baseline-$rev.yml")
  node /node/node_modules/@modelcontextprotocol/conformance/dist/index.js server \
    --url "http://127.0.0.1:$PROXY_PORT/mcp" --requirements "$rev" "${baseline[@]}" -o "/results/$rev" || rc=1
done
echo "== sdk-client"
cp /mcp6/sdk-client.mjs /node/sdk-client.mjs
node /node/sdk-client.mjs || rc=1
kill $proxy
exit $rc
SCRIPT
grep -E "^(== |PASS|FAIL|sdk-client:)|passed|failed|Expected|Unexpected|stale" "$log" | tail -80 || true
if [ "$status" -eq 0 ]; then say "mcp-conformance: OK (results in $work/results)"; else say "mcp-conformance: FAILED (see $log)"; fi
exit "$status"
