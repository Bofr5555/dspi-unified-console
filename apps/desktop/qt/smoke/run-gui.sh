#!/bin/bash
# The normal DSPiConsole on your desktop session, from the build container, to try
# by hand: the signal generator and the rest. This is the real app with write access
# to the device; nothing guards what you click. The signal generator plays on the
# device itself, so no audio is forwarded.
#
#   apps/desktop/qt/smoke/run-gui.sh            the first DSPi on USB
#   apps/desktop/qt/smoke/run-gui.sh --demo     the demo device first
#   DSPI_GUI=x11 apps/desktop/qt/smoke/run-gui.sh    X11 even on a Wayland session
#
# What it forwards: the Wayland socket (or the X11 socket and its Xauthority), all
# of /dev/bus/usb and real sysfs, so a replug works. No network, so AutoEQ's rebuild
# fails; no D-Bus, so file dialogs are Qt's own. No GPU: Qt Quick's software backend,
# so the host's graphics driver doesn't matter. Settings persist in
# ~/.cache/dspi-build/work/<branch>/home-gui, apart from your own Console's.
set -euo pipefail
cd "$(dirname "$0")/../../../.."
B=${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}
branch=$(git rev-parse --abbrev-ref HEAD); branch=${branch//\//-}

display=()
wayland=${WAYLAND_DISPLAY:-}
[ -n "$wayland" ] && [ "${wayland#/}" = "$wayland" ] && wayland=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/$wayland
if [ "${DSPI_GUI:-}" != x11 ] && [ -S "$wayland" ]; then
    echo "display: Wayland, $wayland"
    display=(-v "$wayland":/run/display/wayland-0 -e XDG_RUNTIME_DIR=/run/display -e WAYLAND_DISPLAY=wayland-0
             -e QT_QPA_PLATFORM=wayland)
elif [ -n "${DISPLAY:-}" ] && [ -S "/tmp/.X11-unix/X${DISPLAY#*:}" ]; then
    echo "display: X11, $DISPLAY"
    display=(-v /tmp/.X11-unix:/tmp/.X11-unix -e "DISPLAY=$DISPLAY" -e QT_QPA_PLATFORM=xcb -e QT_X11_NO_MITSHM=1)
    if [ -n "${XAUTHORITY:-}" ] && [ -f "$XAUTHORITY" ]; then
        display+=(-v "$XAUTHORITY":/run/display/xauthority:ro -e XAUTHORITY=/run/display/xauthority)
    fi
else
    echo "no display: neither a Wayland socket (WAYLAND_DISPLAY=${WAYLAND_DISPLAY:-}) nor an X11 socket (DISPLAY=${DISPLAY:-})" >&2
    exit 2
fi

ci/check.sh --step qt

home=$B/work/$branch/home-gui
if [ ! -f "$home/.config/DSPi/DSPi Console.conf" ]; then
    mkdir -p "$home/.config/DSPi"
    printf '[onboarding]\nsetupDone=true\n' > "$home/.config/DSPi/DSPi Console.conf"
fi

# As run-replug.sh: the whole of /dev/bus/usb, real sysfs, no keep-id, no network.
exec podman run --rm -i --init --security-opt label=disable --network=none \
    -v /dev/bus/usb:/dev/bus/usb:dev \
    "${display[@]}" \
    -v "$B/work/$branch":/work \
    -e HOME=/work/home-gui -e TMPDIR=/work/tmp -e QT_QUICK_BACKEND=software -e "RUST_LOG=${RUST_LOG:-warn}" \
    "${DSPI_IMAGE:-localhost/dspi-coremerge:3}" \
    /work/build-desktop-qt/DSPiConsole "$@"
