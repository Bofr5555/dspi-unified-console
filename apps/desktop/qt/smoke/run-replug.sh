#!/bin/bash
# Three unplug and replug cycles on the host's one DSPi, against the real app
# (SmokeDriverReplug.cpp): disconnected state within 2.5 s, no crash, auto-reconnect,
# the image identical after. It writes nothing to the device. Follow the prompts;
# it gives up after 45 s on a step, and 5.5 minutes overall. Close every other DSPi
# program first. If the replug cuts the DSPi's power, save or revert anything
# unsaved beforehand: a reboot drops it, and that shows as an image difference.
#
#   apps/desktop/qt/smoke/run-replug.sh
#
# The log and window images land under ~/.cache/dspi-build/work/<branch>/smoke-replug.
set -euo pipefail
cd "$(dirname "$0")/../../../.."
B=${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}
branch=$(git rev-parse --abbrev-ref HEAD); branch=${branch//\//-}

count=0
for d in /sys/bus/usb/devices/*; do
    [ -f "$d/idVendor" ] && [ -f "$d/idProduct" ] || continue
    case "$(cat "$d/idVendor"):$(cat "$d/idProduct")" in
        2e8b:feaa|2e8a:feaa) count=$((count + 1)) ;;
    esac
done
if [ "$count" != 1 ]; then
    echo "plug in exactly one DSPi first (found $count)" >&2
    exit 2
fi

ci/run.sh "cargo build --release -p dspi-capi --features link,midir,mdns && cmake -S apps/desktop/qt -B /work/build-desktop-qt6-smoke -G Ninja -DCMAKE_BUILD_TYPE=Release -DDSPI_QT_MAJOR=6 -DDSPI_CARGO_TARGET_DIR=/target -DDSPI_SMOKE=ON && cmake --build /work/build-desktop-qt6-smoke --target DSPiConsoleSmoke"

# The node changes on every replug, so the whole of /dev/bus/usb (one devtmpfs, so
# new nodes show up; `dev` because a volume is nodev by default) and real sysfs
# (podman's own; ci/run.sh hides it), not one --device. No keep-id: the container's
# root is the host user the node's ACL names. No network.
exec podman run --rm -i --init --security-opt label=disable --network=none \
    -v /dev/bus/usb:/dev/bus/usb:dev \
    -v "$PWD":/src:ro -v "$B/work/$branch":/work \
    -e HOME=/work/home -e TMPDIR=/work/tmp -e DSPI_SMOKE_HW=replug -w /src \
    "${DSPI_IMAGE:-localhost/dspi-coremerge:2}" \
    apps/desktop/qt/smoke/run.sh /work/build-desktop-qt6-smoke/DSPiConsoleSmoke /work/smoke-replug
