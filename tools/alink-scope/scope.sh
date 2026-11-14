#!/bin/bash
# WP SCOPE1: the alink scope tool in its container (PicoScope 2000A MSO, ps2000a driver).
#
#   scope.sh build                    build the image (downloads from Pico's official repositories)
#   scope.sh test                     unit tests (synthetic edges, fake driver; no device, no network)
#   scope.sh analyse EDGES.csv ARGS   re-analyse a saved capture (no device)
#   scope.sh find                     print the scope's USB node (reads sysfs only, never opens it)
#   scope.sh run --go ARGS            THE SCOPE: passes its USB node into the container and runs
#                                     `python3 -m alink_scope ARGS` (info | capture ...). Only with
#                                     the owner's go; needs the host udev rule (README.md).
#
# Output goes to $ALINK_SCOPE_OUT (default ~/.cache/dspi-build/work/alink-scope), never tmpfs.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
IMAGE=${ALINK_SCOPE_IMAGE:-localhost/dspi-alink-scope:1}
B=${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}
OUT=${ALINK_SCOPE_OUT:-$B/work/alink-scope}
VIDPID=0ce9:1016          # PicoScope 2000A series
mkdir -p "$OUT"
die() { echo "scope.sh: $*" >&2; exit 1; }

node() {
  local nodes=() d
  for d in /sys/bus/usb/devices/*; do
    [ -f "$d/idVendor" ] && [ -f "$d/idProduct" ] || continue
    [ "$(cat "$d/idVendor"):$(cat "$d/idProduct")" = "$VIDPID" ] || continue
    nodes+=("$(printf '/dev/bus/usb/%03d/%03d' "$((10#$(cat "$d/busnum")))" "$((10#$(cat "$d/devnum")))")")
  done
  [ "${#nodes[@]}" = 1 ] || die "expected one PicoScope 2000A ($VIDPID), found ${#nodes[@]}"
  echo "${nodes[0]}"
}

# No device, no network: the tool's code read-only, output in $OUT.
nodev() {
  podman run --rm -i --network none --security-opt label=disable \
    -v "$here":/tool:ro -v "$OUT":/out -e ALINK_SCOPE_TMP=/out/test-tmp -e PYTHONDONTWRITEBYTECODE=1 \
    -w /tool "$IMAGE" "$@"
}

case "${1:-}" in
  build)
    mkdir -p "$OUT/ctx"; podman build -t "$IMAGE" -f "$here/Containerfile" "$OUT/ctx" ;;
  test)
    nodev python3 -m unittest discover -s tests -v ;;
  analyse)
    shift; [ $# -ge 1 ] || die "analyse EDGES.csv [ARGS]"
    f=$(cd "$(dirname "$1")" && pwd)/$(basename "$1"); shift
    case "$f" in "$OUT"/*) ;; *) die "put the capture under $OUT" ;; esac
    nodev python3 -m alink_scope analyse "/out/${f#"$OUT"/}" "$@" ;;
  find)
    node ;;
  run)
    shift; [ "${1:-}" = "--go" ] || die "run needs --go (the owner's go to open the scope)"; shift
    dev=$(node)
    [ -r "$dev" ] && [ -w "$dev" ] || die "$dev is not read/writable by $(id -un): install the udev rule (README.md)"
    # Rootless: the container's root is this user, so the node's uaccess ACL (or group, with
    # keep-groups) is what grants access. Only the scope's own node goes in.
    podman run --rm -it --network none --security-opt label=disable \
      --device "$dev" --group-add keep-groups \
      -v "$here":/tool:ro -v "$OUT":/out -w /out -e PYTHONPATH=/tool -e PYTHONDONTWRITEBYTECODE=1 \
      "$IMAGE" python3 -m alink_scope "$@" ;;
  *)
    sed -n '2,13p' "$0"; exit 2 ;;
esac
