#!/bin/bash
# WP LK3: the opt-in CI step for the emulator rig (ci/check.sh --step esp-rig; not in
# --all). Builds what is missing (firmware, tunnel, our probe), saves a 1 Mbaud
# bridge image, then runs `scenarios.sh smoke` with our client and upstream's TS
# client against the emulated bridge and the demo device: both must reach live
# meters, with no session lost and no bridge 10 s close. Needs the DSPi-Link clone
# in work/consoles (as conformance-ts). Takes about five minutes the first time.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
work=${ESP_RIG_WORK:-${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}/work/esp-rig}
export ESP_RIG_WORK=$work
"$here/rig.sh" build
[ -f "$work/fw/flash-1000000.bin" ] || "$here/rig.sh" provision 1000000
log=$work/ci-smoke.log
CLIENTS="after ts" "$here/scenarios.sh" smoke | tee "$log"
results=$(grep '^RESULT' "$log" || true)
[ "$(wc -l <<< "$results")" -ge 2 ] || { echo "esp-rig: missing results" >&2; exit 1; }
bad=$(grep -E ' (first_meter=never|failed=[^n]|drops=[1-9]|closes=[1-9]|unwritable_closes=[1-9])' <<< "$results" || true)
[ -z "$bad" ] || { echo "esp-rig: FAILED:" >&2; echo "$bad" >&2; exit 1; }
echo "esp-rig: OK (both clients reached live meters over the emulated bridge)"
