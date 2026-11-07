#!/bin/bash
# WP HB7: the TypeScript or the Swift client through the shared scenario list,
# against this hub (README.md). Our own client is `cargo test --test conformance`.
#
#   crates/dspi-hub/tests/conformance/run-clients.sh ts      upstream's @dspi/link in Node 24, in a container
#   crates/dspi-hub/tests/conformance/run-clients.sh swift   the Swift console's LinkClient, in the macOS VM
#
# The hub is the conformance test binary's ignored `serve` test, built by ci/run.sh
# and run in a container from the same image, with no USB. The client prints
# `@ctl <command>` for what only the hub's side can do; this script passes it on
# through files (serve.rs) and answers with the hub's JSON line.
#
# ts:    Node from the official image, pinned by digest (NODE in
#        ci/container/images.conf; NODE_IMAGE overrides it). The client is
#        upstream's source at DSPI_LINK_COMMIT, taken from a clone (DSPI_LINK_REPO)
#        with `git archive` and checked against its tree hash: no npm download. The
#        two containers share one network namespace; nothing is published.
# swift: ssh to DSPI_MACVM (default macvm). The client's sources and these files go
#        to ~/work/matrix/hb7 there and are built with swiftc, under the matrix's
#        lock (~/work/matrix/lock, as ci/macvm.sh), so no xcodebuild runs beside
#        it. The hub's port is published on the host's loopback only and reaches
#        the guest through `ssh -R`.
#
# Scratch and logs: ${DSPI_BUILD_CACHE:-~/.cache/dspi-build}/work/hb7-conformance/<client>/,
# never tmpfs. Exit status: the client's (0 when every scenario passed); 3 for
# swift when the VM cannot run it (no ssh, or nobody logged in to its GUI), as
# ci/macvm.sh.
set -euo pipefail
client=${1:-}
case "$client" in ts|swift) ;; *) echo "usage: $0 ts|swift" >&2; exit 2 ;; esac
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../../.." && pwd)
. "$root/ci/container/images.conf"
B=${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}
NODE_IMAGE=${NODE_IMAGE:-$NODE}
DSPI_LINK_REPO=${DSPI_LINK_REPO:-$B/work/consoles/DSPi-Link}
DSPI_LINK_COMMIT=86f10e38dd760d01d4f589e4b606a5250b2d523d
# git rev-parse 86f10e38:js/packages/dspi-link
DSPI_LINK_TREE=ac3f823b0d8d638ea133807da431fd9f4e2d8ebd
host=${DSPI_MACVM:-macvm}
work=$B/work/hb7-conformance/$client
rm -rf "$work" && mkdir -p "$work/conf/tmp"
log=$work/run.log
: >"$log"
say() { echo "$*" | tee -a "$log"; }
vm() { ssh -o ConnectTimeout=10 -o BatchMode=yes "$host" "$@"; }

# -- WP HB8 -- 0. The VM can run the Swift client: checked before anything is built.
if [ "$client" = swift ]; then
  who=$(vm who </dev/null 2>&1) || { say "run-clients: $host is not reachable: $who"; exit 3; }
  grep -qw console <<<"$who" || { say "run-clients: nobody is logged in to the GUI of $host"; exit 3; }
fi

# 1. The hub's binary.
say "building the conformance test binary"
bin=$("$root/ci/run.sh" 'cargo test -p dspi-hub --test conformance --no-run 2>&1' |
  tee -a "$log" | sed -n 's/.*Executable tests\/conformance\/main.rs (\(.*\))$/\1/p' | tail -1)
[ -n "$bin" ] || { say "no test binary (see $log)"; exit 1; }
branch=$(git -C "$root" rev-parse --abbrev-ref HEAD 2>/dev/null || echo detached)
target=$B/target/${branch//\//-}

# 2. The hub, in its own container.
name=hb7-serve-$client-$$
publish=()
if [ "$client" = swift ]; then publish=(-p 127.0.0.1::11915); fi
cleanup() {
  podman rm -f "$name" >/dev/null 2>&1 || true
  if [ -n "${tunnel_pid:-}" ]; then kill "$tunnel_pid" 2>/dev/null || true; fi
}
trap cleanup EXIT
podman run -d --name "$name" --userns=keep-id --user "$(id -u):$(id -g)" \
  -v "$target":/target:ro,z -v "$work/conf":/conf:z -v "$B/work/empty":/sys/bus/usb/devices:ro,z \
  -e HB7_CONF_DIR=/conf -e TMPDIR=/conf/tmp -e HB7_BIND=0.0.0.0 -e HB7_PORT=11915 \
  "${publish[@]}" "$COREMERGE" \
  "$bin" --ignored --exact serve::conformance_serve_for_other_clients --nocapture >/dev/null
for _ in $(seq 200); do [ -s "$work/conf/hub.json" ] && break; sleep 0.1; done
[ -s "$work/conf/hub.json" ] || { podman logs "$name" >>"$log" 2>&1; say "the hub did not start (see $log)"; exit 1; }
hub_json=$(cat "$work/conf/hub.json")
field() { sed -n "s/.*\"$1\":\\(\"\\([^\"]*\\)\"\\|\\([0-9]*\\)\\).*/\\2\\3/p" <<<"$hub_json"; }
hub_id=$(field hub_id)
pin=$(field pin)
say "hub $hub_id up"

# 3. The client's command.
if [ "$client" = ts ]; then
  up=$work/upstream
  mkdir -p "$up"
  got=$(git -C "$DSPI_LINK_REPO" rev-parse "$DSPI_LINK_COMMIT:js/packages/dspi-link")
  [ "$got" = "$DSPI_LINK_TREE" ] || { say "upstream tree $got, expected $DSPI_LINK_TREE"; exit 1; }
  git -C "$DSPI_LINK_REPO" archive "$DSPI_LINK_COMMIT" js/packages/dspi-link/src | tar -x -C "$up"
  run=(podman run --rm -i --network="container:$name" --userns=keep-id --user "$(id -u):$(id -g)"
    -v "$up":/upstream:ro,z -v "$here/ts":/ts:ro,z
    -e HB7_URL=ws://127.0.0.1:11915/dspi/v1 -e "HB7_PIN=$pin" -e "HB7_HUB_ID=$hub_id"
    "$NODE_IMAGE" node --experimental-transform-types --no-warnings --import /ts/register.mjs /ts/conformance.ts)
else
  port=$(podman port "$name" 11915/tcp | sed 's/.*://')
  apple="$root/apps/apple/DSPi Console/Link"
  sources=(LinkClient.swift LinkClientProtocol.swift LinkFrames.swift LinkMessages.swift LinkRole.swift
    DeviceTransport.swift AuthStore.swift)
  stage=$work/swift
  mkdir -p "$stage"
  for f in "${sources[@]}"; do cp "$apple/$f" "$stage/"; done
  cp "$here/swift/main.swift" "$here/swift/Shims.swift" "$stage/"
  # Copy first; then one session takes the matrix's lock with its own pid, builds,
  # runs and lets go, so a lock left by a lost session is seen as stale.
  tar -C "$stage" -cf - . | vm 'rm -rf ~/work/matrix/hb7 && mkdir -p ~/work/matrix/hb7 && tar -xf - -C ~/work/matrix/hb7'
  # The hub's port at the same number on the guest's loopback.
  ssh -o BatchMode=yes -o ExitOnForwardFailure=yes -N -R "127.0.0.1:$port:127.0.0.1:$port" "$host" &
  tunnel_pid=$!
  run=(ssh -o BatchMode=yes "$host" "set -u
    while ! mkdir ~/work/matrix/lock 2>/dev/null; do
      pid=\$(cat ~/work/matrix/lock/pid 2>/dev/null)
      if [ -n \"\$pid\" ] && ! kill -0 \"\$pid\" 2>/dev/null; then rm -rf ~/work/matrix/lock; continue; fi
      echo \"waiting for the run holding ~/work/matrix/lock (pid \$pid)\"; sleep 30
    done
    echo \$\$ >~/work/matrix/lock/pid
    trap 'rm -rf ~/work/matrix/lock' EXIT
    cd ~/work/matrix/hb7 && echo 'building with swiftc' &&
      xcrun swiftc -O -o conformance *.swift 2>&1 | grep -E 'error' ; test -x conformance || exit 1
    HB7_URL=ws://127.0.0.1:$port/dspi/v1 HB7_HUB_ID=$hub_id ./conformance")
fi

# 4. Run it, answering its requests.
n=0
reply=
answer() {
  n=$((n + 1))
  reply='{"error":"no answer from the hub"}'
  printf '%s %s\n' "$n" "$1" >"$work/conf/.cmd.tmp" && mv "$work/conf/.cmd.tmp" "$work/conf/cmd"
  local ack
  for _ in $(seq 500); do
    ack=$(cat "$work/conf/ack" 2>/dev/null || true)
    if [ "${ack%% *}" = "$n" ]; then
      rm -f "$work/conf/ack"
      reply=${ack#* }
      return
    fi
    sleep 0.02
  done
}
coproc CLIENT { "${run[@]}" 2>&1; }
client_pid=$CLIENT_PID
# Bash unsets CLIENT when the client exits: keep its descriptors.
exec {to_client}>&"${CLIENT[1]}" {from_client}<&"${CLIENT[0]}"
while IFS= read -r line <&"$from_client"; do
  say "$line"
  case "$line" in
  "@ctl "*)
    answer "${line#@ctl }"
    echo "$reply" >>"$log"
    echo "$reply" >&"$to_client"
    ;;
  esac
done
wait "$client_pid" && status=0 || status=$?
podman logs "$name" >>"$log" 2>&1 || true
say "client exited $status; log $log"
exit "$status"
