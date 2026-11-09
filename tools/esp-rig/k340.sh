#!/bin/bash
# WP LK3: read-only state dump of the real DSPi (the K340) on the host, before and
# after a rig run with `--device usb`, and their diff (the K340 rules: backup,
# compare after; never flash, no siggen, no VREG writes). Every opcode below is a
# GET; the tunnel itself refuses SETs unless given --allow-writes.
#
#   k340.sh dump DIR      write DIR/ (the Terminal client, `rig.sh build` with ESP_RIG_DSPI_CLI=1)
#   k340.sh compare A B   exit 0 when identical (timestamps and uptime ignored)
#
# Same method as the hw-1 job: export, filters, control surfaces, the bulk image,
# names, presets, VREG (read only), the crossfeed map.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
. "$root/ci/container/images.conf"
work=${ESP_RIG_WORK:-${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}/work/esp-rig}

node() {
  local nodes=() d
  for d in /sys/bus/usb/devices/*; do
    [ -f "$d/idVendor" ] && [ -f "$d/idProduct" ] || continue
    case "$(cat "$d/idVendor"):$(cat "$d/idProduct")" in
      2e8b:feaa) nodes+=("$(printf '/dev/bus/usb/%03d/%03d' "$(cat "$d/busnum")" "$(cat "$d/devnum")")") ;;
      2e8a:*) echo "k340.sh: a board is in its bootloader: refusing" >&2; exit 3 ;;
    esac
  done
  [ "${#nodes[@]}" = 1 ] || { echo "k340.sh: expected one DSPi, found ${#nodes[@]}" >&2; exit 2; }
  echo "${nodes[0]}"
}

dump() {
  local out=$1
  mkdir -p "$out"
  [ -x "$work/dspi" ] || { echo "k340.sh: no $work/dspi (ESP_RIG_DSPI_CLI=1 rig.sh build)" >&2; exit 2; }
  podman run --rm -i --security-opt label=disable --network none --device "$(node)" \
    -v "$work":/rig:ro -v "$out":/out:z -e HOME=/out "$COREMERGE" bash -s <<'EOF'
set -u
D=/rig/dspi; out=/out
$D export "$out/device.dspipreset" --format preset > "$out/export.log" 2>&1; echo "export rc=$?" > "$out/rc.txt"
$D export "$out/filters.txt" --format filters >> "$out/export.log" 2>&1; echo "filters rc=$?" >> "$out/rc.txt"
$D cs export "$out/cs.json" >> "$out/export.log" 2>&1; echo "cs rc=$?" >> "$out/rc.txt"
$D raw 0xA0 6136 --json > "$out/bulk_A0.json" 2>&1; echo "bulk rc=$?" >> "$out/rc.txt"
{
  echo "dev.name: $($D raw 0x57 32 --json)"
  for s in $(seq 0 15); do echo "in.name[$s]: $($D raw 0x55 32 $s --json 2>&1)"; done
  for c in $(seq 0 16); do echo "ch.name[$c]: $($D raw 0x9C 32 $c --json 2>&1)"; done
  for p in $(seq 0 9); do echo "preset.name[$p]: $($D raw 0x93 32 $p --json 2>&1)"; done
  echo "preset.dir: $($D raw 0x95 64 --json 2>&1)"
  echo "preset.startup: $($D raw 0x97 8 --json 2>&1)"
  echo "preset.active: $($D raw 0x9A 8 --json 2>&1)"
  echo "uart.config: $($D raw 0xF6 8 --json 2>&1)"
} > "$out/names_raw.txt"
{
  echo "## ext vreg (read only; uptime_s varies)"; $D ext vreg
  echo "## ext xfeed-map"; $D ext xfeed-map
} > "$out/readable.txt" 2>&1
EOF
  ls "$out"
}

compare() {
  local a=$1 b=$2 rc=0 f
  norm() {
    case "$1" in
      *.dspipreset | *.json) python3 -I -c 'import json,sys
def strip(o):
  if isinstance(o,dict): return {k:strip(v) for k,v in o.items() if k not in ("savedUtc",)}
  if isinstance(o,list): return [strip(x) for x in o]
  return o
print(json.dumps(strip(json.load(open(sys.argv[1]))),indent=1,sort_keys=True))' "$1" ;;
      *) grep -v -e '^# Exported:' -e 'uptime' "$1" ;;
    esac
  }
  for f in device.dspipreset bulk_A0.json cs.json filters.txt names_raw.txt readable.txt rc.txt; do
    if diff <(norm "$a/$f") <(norm "$b/$f") > /dev/null; then
      echo "same  $f"
    else
      echo "DIFF  $f"; diff <(norm "$a/$f") <(norm "$b/$f") | head -40; rc=1
    fi
  done
  return $rc
}

case "${1:-}" in
  dump) dump "${2:?dir}" ;;
  compare) compare "${2:?a}" "${3:?b}" ;;
  *) sed -n '2,12p' "$0" >&2; exit 2 ;;
esac
