#!/bin/bash
# The hardware pass of the smoke driver, on the host's one DSPi, in a container
# that sees only that USB device node. See SmokeDriverHw.cpp for what it does.
# Close every other DSPi program first, and stop playback: it toggles global bypass
# for under a second, once per check.
#
#   apps/desktop/qt/smoke/run-hw.sh
#
# The log and window images land under ~/.cache/dspi-build/work/<branch>/smoke-hw.
set -euo pipefail
cd "$(dirname "$0")/../../../.."
B=${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}
branch=$(git rev-parse --abbrev-ref HEAD); branch=${branch//\//-}

nodes=()
for d in /sys/bus/usb/devices/*; do
    [ -f "$d/idVendor" ] && [ -f "$d/idProduct" ] || continue
    case "$(cat "$d/idVendor"):$(cat "$d/idProduct")" in
        2e8b:feaa|2e8a:feaa) nodes+=("$(printf '/dev/bus/usb/%03d/%03d' "$(cat "$d/busnum")" "$(cat "$d/devnum")")") ;;
    esac
done
if [ "${#nodes[@]}" != 1 ]; then
    echo "expected exactly one DSPi on USB, found ${#nodes[@]}: ${nodes[*]:-}" >&2
    exit 2
fi
node=${nodes[0]}
echo "device node: $node"

# The build has no device: the usual container
ci/run.sh "cargo build --release -p dspi-capi && cmake -S apps/desktop/qt -B /work/build-desktop-qt6-smoke -G Ninja -DCMAKE_BUILD_TYPE=Release -DDSPI_QT_MAJOR=6 -DDSPI_CARGO_TARGET_DIR=/target -DDSPI_SMOKE=ON && cmake --build /work/build-desktop-qt6-smoke --target DSPiConsoleSmoke"

# The run: real sysfs (ci/run.sh hides it), the one node, no network. No keep-id:
# the container's root is the host user the node's ACL names.
exec podman run --rm -i --security-opt label=disable \
    --network=none --device "$node" \
    -v "$PWD":/src:ro -v "$B/work/$branch":/work \
    -e HOME=/work/home -e TMPDIR=/work/tmp -e DSPI_SMOKE_HW=1 -w /src \
    "${DSPI_IMAGE:-localhost/dspi-coremerge:2}" \
    apps/desktop/qt/smoke/run.sh /work/build-desktop-qt6-smoke/DSPiConsoleSmoke /work/smoke-hw
