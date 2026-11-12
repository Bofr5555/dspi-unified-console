#!/bin/bash
# WP LK3: the emulated ESP32 DSPi-Link bridge rig. See README.md.
#
#   rig.sh build                      firmware (if missing), tunnel, our probe
#                                     (ESP_RIG_RELEASE=1: the probe optimised)
#   rig.sh build-before               our probe against the pre-LK2 core (4cdefa24)
#   rig.sh provision BAUD             boot once, save the bridge's UART rate (detection off)
#                                     into its flash: fw/flash-BAUD.bin
#   rig.sh up RUN BAUD [TUNNEL ARGS]  start a rig: network namespace, bridge, tunnel
#   rig.sh shape RUN [NETEM ARGS]     shape both directions (nothing: clear)
#   rig.sh outage RUN SECONDS         drop everything for SECONDS, then the old shaping
#   rig.sh rust RUN [VAR=VALUE...]    our client (crates/dspi-core/tests/wp_lk3_rig.rs)
#   rig.sh rust-before RUN [VAR=...]  the same, built from the pre-LK2 core
#   rig.sh rust-bin RUN BIN [VAR=...] the same, from $ESP_RIG_WORK/BIN (a kept build)
#   rig.sh ts RUN [VAR=VALUE...]      upstream's TypeScript client (clients/ts-probe.ts)
#   rig.sh realhub RUN [FILTER]       crates/dspi-link/tests/real_hub against the bridge
#                                     (default filter real::, the read-only safe set)
#   rig.sh log RUN                    the bridge's serial log
#   rig.sh down RUN                   stop and remove the rig's containers
#
# Every container runs rootless with no published port: the bridge (QEMU's
# open_eth on tap0, 10.77.0.2), the tunnel and the clients share one private
# network namespace whose tap0 (10.77.0.1) and ifb0 carry netem, one per
# direction. Tunnel args: see `esp-rig-tunnel --help`; `--device usb` passes the
# one DSPi on the host into the tunnel (read-only unless --allow-writes).
#
# Work files: $ESP_RIG_WORK (default ~/.cache/dspi-build/work/esp-rig), never tmpfs.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
. "$here/images.conf"
. "$root/ci/container/images.conf"
B=${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}
work=${ESP_RIG_WORK:-$B/work/esp-rig}
# -- WP CLEAN1 -- the Terminal's command name
ci_cmd() { "$root/ci/command-name.sh"; }
fw=$work/fw
# -- WP LK4 -- a target dir of its own, so two rigs (a WP and a review) do not share one
target=${ESP_RIG_TARGET:-$B/target/esp-rig}
NETEM_IMAGE=${NETEM_IMAGE:-localhost/lk3-netem:1}
FEDORA_BASE=registry.fedoraproject.org/fedora:44@sha256:5b3a1aeece15ae01e3d04541e9269226d40e832566f3ce183856a12951dddb1e
# -- WP LK4 -- (review c73 N3) containers that run a client or the demo device see no USB
NO_USB=(-v "$B/work/empty":/sys/bus/usb/devices:ro,z)
QEMU=/opt/esp/tools/qemu-xtensa/esp_develop_9.2.2_20260417/qemu/bin/qemu-system-xtensa
BRIDGE=10.77.0.2
URL=ws://$BRIDGE:11915/dspi/v1
BEFORE=4cdefa24 # main before LK1/LK2 (2845510d^), the 0.12.0 client
mkdir -p "$work" "$B/work/empty"

die() { echo "rig.sh: $*" >&2; exit 1; }
put() { cp "$1" "$2.new" && mv -f "$2.new" "$2"; } # a running rig keeps its old binary
run_dir() { echo "$work/runs/$1"; }
need_run() { [ -f "$(run_dir "$1")/ids" ] || die "no rig $1 (rig.sh up $1 ...)"; }
net_of() { echo "lk3-$1-net"; }

netem_image() {
  podman image exists "$NETEM_IMAGE" && return
  # Fedora 44 with iproute and tc from the Fedora repositories (as LK1's lk1-netem:1).
  # -- WP LK4 -- (review c73 N1) the base pinned by digest
  printf 'FROM %s\nRUN dnf -y install iproute-tc iproute && dnf clean all\n' "$FEDORA_BASE" |
    podman build -q -t "$NETEM_IMAGE" -f - "$work" > /dev/null
}

cargo_in() { # cargo in the build image, target dir $1, source $2
  local target=$1 src=$2; shift 2
  mkdir -p "$target" "$B/cargo-home" "$work/home"
  podman run --rm -i --userns=keep-id --user "$(id -u):$(id -g)" \
    -v "$src":/src:z -v "$target":/target:z -v "$B/cargo-home":/cache/cargo:z \
    -v "$work":/work:z -v "$B/work/empty":/sys/bus/usb/devices:ro,z \
    -e CARGO_HOME=/cache/cargo -e CARGO_TARGET_DIR=/target -e HOME=/work/home \
    -e CARGO_BUILD_JOBS="${DSPI_JOBS:-4}" -w /src "$COREMERGE" bash -c "$*"
}

probe_bin() { # build the probe test in $1 (target) from $2 (source), print its path on the host
  local out
  # -- WP LK4 -- ESP_RIG_RELEASE=1: the optimised build, as the apps ship it
  local profile=""
  [ "${ESP_RIG_RELEASE:-0}" = 1 ] && profile=--release
  out=$(cargo_in "$1" "$2" "cargo test $profile -p dspi-core --features link --test wp_lk3_rig --no-run 2>&1") ||
    { echo "$out" >&2; die "probe build failed"; }
  local bin
  bin=$(sed -n 's/.*Executable tests\/wp_lk3_rig.rs (\/target\/\(.*\))$/\1/p' <<< "$out" | tail -1)
  [ -n "$bin" ] || { echo "$out" >&2; die "no probe binary"; }
  echo "$1/$bin"
}

cmd_build() {
  netem_image
  [ -f "$fw/flash.bin" ] || "$here/firmware/build.sh" "$fw"
  cargo_in "$target" "$root" 'cargo build --release -p dspi-esp-rig 2>&1 | tail -1'
  put "$target/release/esp-rig-tunnel" "$work/esp-rig-tunnel"
  put "$(probe_bin "$target" "$root")" "$work/probe-after"
  local out bin
  out=$(cargo_in "$target" "$root" 'cargo test -p dspi-link --test real_hub --no-run 2>&1') ||
    { echo "$out" >&2; die "real_hub build failed"; }
  bin=$(sed -n 's/.*Executable tests\/real_hub\/main.rs (\/target\/\(.*\))$/\1/p' <<< "$out" | tail -1)
  put "$target/$bin" "$work/real-hub-test"
  # The release Terminal client, for the K340 backup and diff (README).
  if [ "${ESP_RIG_DSPI_CLI:-0}" = 1 ]; then
    cargo_in "$target" "$root" 'cargo build --release -p dspi-term 2>&1 | tail -1'
    put "$target/release/$(ci_cmd)" "$work/$(ci_cmd)"
  fi
  echo "built: $work/esp-rig-tunnel, $work/probe-after"
}

cmd_build_before() {
  local src=$work/before-src
  rm -rf "$src" && mkdir -p "$src"
  git -C "$root" archive "$BEFORE" | tar -x -C "$src"
  cp "$root/crates/dspi-core/tests/wp_lk3_rig.rs" "$src/crates/dspi-core/tests/"
  put "$(probe_bin "$B/target/esp-rig-before" "$src")" "$work/probe-before"
  echo "built: $work/probe-before (core at $BEFORE)"
}

ids_add() { echo "$2" >> "$(run_dir "$1")/ids"; }

cmd_up() {
  local run=$1 baud=$2; shift 2
  local dir; dir=$(run_dir "$run")
  [ -f "$dir/ids" ] && die "rig $run is up (rig.sh down $run)"
  local image=$fw/flash-$baud.bin
  # -- WP LK4 -- (review c83 N6) a rig at a rate the bridge was not provisioned for
  # would auto-detect another rate and give bogus results: refuse. Only the
  # provisioning run itself boots the plain image.
  if [ ! -f "$image" ]; then
    [[ $run == prov-* ]] || die "no bridge image saved at $baud baud: run 'rig.sh provision $baud' first"
    image=$fw/flash.bin
  fi
  [ -f "$image" ] || die "no firmware (rig.sh build)"
  [ -x "$work/esp-rig-tunnel" ] || die "no tunnel (rig.sh build)"
  netem_image
  rm -rf "$dir" && mkdir -p "$dir"
  : > "$dir/ids"
  cp "$image" "$dir/flash.bin"
  echo "$image" > "$dir/image"
  local net; net=$(net_of "$run")
  ids_add "$run" "$(podman run -d --init --name "$net" --cap-add NET_ADMIN --network none \
    --security-opt label=disable --device /dev/net/tun "$NETEM_IMAGE" sleep infinity)"
  podman exec "$net" bash -euc '
    ip link set lo up
    ip tuntap add dev tap0 mode tap
    ip addr add 10.77.0.1/24 dev tap0
    ip link set tap0 up
    ip link add ifb0 type ifb
    ip link set ifb0 up
    tc qdisc add dev tap0 handle ffff: ingress
    tc filter add dev tap0 parent ffff: protocol all matchall action mirred egress redirect dev ifb0'
  ids_add "$run" "$(podman run -d --name "lk3-$run-qemu" --network "container:$net" \
    --security-opt label=disable --device /dev/net/tun -v "$dir":/run:z --entrypoint "$QEMU" \
    "$IDF_IMAGE" -nographic -machine esp32 -drive file=/run/flash.bin,if=mtd,format=raw \
    -nic tap,ifname=tap0,script=no,downscript=no,model=open_eth \
    -serial mon:stdio -serial unix:/run/uart1.sock,server=on,wait=off)"
  local dev=("${NO_USB[@]}") device=demo a
  for a in "$@"; do case "$a" in usb|usb:*) device=$a ;; esac; done
  if [[ $device == usb* ]]; then
    local node; node=$(dspi_node) || die "$node"
    dev=(--device "$node")
  fi
  ids_add "$run" "$(podman run -d --init --name "lk3-$run-tunnel" --network "container:$net" \
    --security-opt label=disable "${dev[@]}" -v "$dir":/run:z -v "$work":/rig:ro "$NETEM_IMAGE" \
    bash -c "exec /rig/esp-rig-tunnel --uart /run/uart1.sock --baud $baud --log /run/tunnel.log --stats-every 10 $* 2> /run/tunnel.err")"
  local until=$((SECONDS + 90))
  until podman logs "lk3-$run-qemu" 2>&1 | grep -q "Found DSPi"; do
    ((SECONDS < until)) || { cmd_log "$run" | tail -20; die "the bridge found no DSPi in 90 s"; }
    sleep 1
  done
  echo "rig $run up: bridge $URL, UART $baud baud, $(podman logs "lk3-$run-qemu" 2>&1 | grep -o 'Found DSPi.*' | tail -1)"
}

dspi_node() { # the one DSPi's USB node, refusing a board in its bootloader (as hw-1)
  local nodes=() d
  for d in /sys/bus/usb/devices/*; do
    [ -f "$d/idVendor" ] && [ -f "$d/idProduct" ] || continue
    case "$(cat "$d/idVendor"):$(cat "$d/idProduct")" in
      2e8b:feaa) nodes+=("$(printf '/dev/bus/usb/%03d/%03d' "$(cat "$d/busnum")" "$(cat "$d/devnum")")") ;;
      2e8a:*) echo "a board is in its bootloader: refusing"; return 1 ;;
    esac
  done
  [ "${#nodes[@]}" = 1 ] || { echo "expected one DSPi on USB, found ${#nodes[@]}"; return 1; }
  echo "${nodes[0]}"
}

cmd_shape() {
  local run=$1; shift
  need_run "$run"
  local net; net=$(net_of "$run")
  echo "$*" > "$(run_dir "$run")/shape"
  podman exec "$net" bash -c "
    tc qdisc del dev tap0 root 2>/dev/null; tc qdisc del dev ifb0 root 2>/dev/null
    if [ -n '$*' ]; then tc qdisc add dev tap0 root netem $* && tc qdisc add dev ifb0 root netem $*; fi"
}

cmd_outage() {
  local run=$1 s=$2
  need_run "$run"
  local old net; old=$(cat "$(run_dir "$run")/shape" 2>/dev/null || true); net=$(net_of "$run")
  echo "rig $run: outage ${s}s from $(date +%T.%N | cut -c1-12)"
  podman exec "$net" bash -c "
    tc qdisc del dev tap0 root 2>/dev/null; tc qdisc del dev ifb0 root 2>/dev/null
    tc qdisc add dev tap0 root netem loss 100% && tc qdisc add dev ifb0 root netem loss 100%"
  sleep "$s"
  cmd_shape "$run" $old
  echo "rig $run: outage over at $(date +%T.%N | cut -c1-12), shaping '${old}'"
}

client_env() { local e=() v; for v in "$@"; do e+=(-e "$v"); done; printf '%s\n' "${e[@]}"; }

cmd_rust() {
  local run=$1 bin=$2; shift 2
  need_run "$run"
  [ -x "$work/$bin" ] || die "no $bin (rig.sh build / build-before)"
  local env=(); mapfile -t env < <(client_env "LK3_URL=$URL" "$@")
  podman run --rm --init --network "container:$(net_of "$run")" --security-opt label=disable \
    "${env[@]}" -v "$work":/rig:ro "${NO_USB[@]}" "$NETEM_IMAGE" \
    "/rig/$bin" --ignored --nocapture --test-threads=1 --exact wp_lk3_meters_against_the_emulated_bridge 2>&1 |
    sed 's/^test wp_lk3_[a-z_]* \.\.\. //' | grep -E '^LK3|panicked|test result' | sed "s/^/[$bin] /"
}

cmd_ts() {
  local run=$1; shift
  need_run "$run"
  local env=(); mapfile -t env < <(client_env "LK3_URL=$URL" "$@")
  podman image exists "$NODE" || podman pull -q "$NODE" > /dev/null
  podman run --rm --init --network "container:$(net_of "$run")" --security-opt label=disable \
    "${env[@]}" -v "$fw/src":/upstream:ro -v "$here/clients":/clients:ro "${NO_USB[@]}" \
    -v "$root/crates/dspi-hub/tests/conformance/ts":/ts:ro "$NODE" \
    node --experimental-transform-types --no-warnings --import /ts/register.mjs /clients/ts-probe.ts 2>&1 |
    sed 's/^/[ts] /'
}

cmd_realhub() {
  local run=$1 filter=${2:-real::}
  need_run "$run"
  [ -x "$work/real-hub-test" ] || die "no real_hub test (rig.sh build)"
  podman run --rm --init --network "container:$(net_of "$run")" --security-opt label=disable \
    -v "$work":/rig:z "${NO_USB[@]}" -e DSPI_REAL_HUB=$BRIDGE:11915 -e DSPI_REAL_HUB_REPORT="/rig/runs/$run/real-hub.jsonl" \
    "$NETEM_IMAGE" /rig/real-hub-test "$filter" --test-threads=1 2>&1 | grep -E '^test |panicked|^test result|endpoint'
}

cmd_provision() {
  local baud=$1 run=prov-$1
  [ -f "$fw/flash.bin" ] || die "no firmware (rig.sh build)"
  rm -f "$fw/flash-$baud.bin"
  cmd_up "$run" "$baud" > /dev/null
  cmd_ts "$run" LK3_MODE=config "LK3_BAUD=$baud"
  local until=$((SECONDS + 60))
  until podman logs "lk3-$run-qemu" 2>&1 | grep -q "Found DSPi.* at $baud baud"; do
    ((SECONDS < until)) || { cmd_down "$run"; die "the bridge did not move to $baud baud"; }
    sleep 1
  done
  sleep 3 # settings are saved before the move; give NVS a moment
  podman stop -t 5 "lk3-$run-qemu" > /dev/null
  cp "$(run_dir "$run")/flash.bin" "$fw/flash-$baud.bin"
  cmd_down "$run"
  echo "provisioned: $fw/flash-$baud.bin (UART $baud baud, detection off)"
}

cmd_log() { need_run "$1"; podman logs "lk3-$1-qemu" 2>&1; }

cmd_down() {
  local run=$1 dir id
  dir=$(run_dir "$run")
  [ -f "$dir/ids" ] || return 0
  podman logs "lk3-$run-qemu" > "$dir/bridge.log" 2>&1 || true # kept for evidence
  tac "$dir/ids" | while read -r id; do
    [ -n "$id" ] && podman rm -f -t 3 "$id" > /dev/null 2>&1 || true
  done
  rm -f "$dir/ids"
}

case "${1:-}" in
  build) cmd_build ;;
  build-before) cmd_build_before ;;
  provision) cmd_provision "${2:?baud}" ;;
  up) shift; cmd_up "${1:?run}" "${2:?baud}" "${@:3}" ;;
  shape) shift; cmd_shape "${1:?run}" "${@:2}" ;;
  outage) cmd_outage "${2:?run}" "${3:?seconds}" ;;
  rust) shift; cmd_rust "${1:?run}" probe-after "${@:2}" ;;
  rust-before) shift; cmd_rust "${1:?run}" probe-before "${@:2}" ;;
  rust-bin) shift; cmd_rust "${1:?run}" "${2:?binary}" "${@:3}" ;;
  ts) shift; cmd_ts "${1:?run}" "${@:2}" ;;
  realhub) cmd_realhub "${2:?run}" "${3:-}" ;;
  log) cmd_log "${2:?run}" ;;
  down) cmd_down "${2:?run}" ;;
  *) sed -n '2,22p' "$0" >&2; exit 2 ;;
esac
