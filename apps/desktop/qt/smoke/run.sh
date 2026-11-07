#!/bin/bash
# Run DSPiConsoleSmoke offscreen on the demo device (ci/check.sh --step
# qt-offscreen). Passes when the driver reports no failure and the core logged
# no warning more than a few times over the run (a polled call that fails is
# reported once, not at the poll rate).
#
#   run.sh <DSPiConsoleSmoke> <output dir: log.txt and one PNG per window>
#
# DSPI_SMOKE_HW=1 runs the hardware pass on the real device instead (run-hw.sh
# sets it up); DSPI_SMOKE_HW=dry runs that same pass on the demo device;
# DSPI_SMOKE_HW=replug waits for the owner to replug the device (run-replug.sh),
# printing as it goes.
set -uo pipefail
app=$1; out=$2
rm -rf "$out"; mkdir -p "$out/home"
export HOME="$out/home" XDG_CONFIG_HOME="$out/home/.config"
mkdir -p "$HOME/.config/DSPi"
printf '[onboarding]\nsetupDone=true\n' > "$HOME/.config/DSPi/DSPi Console.conf"
export QT_QPA_PLATFORM=offscreen QT_QUICK_BACKEND=software QT_FORCE_STDERR_LOGGING=1
export RUST_LOG=warn DSPI_SMOKE_OUT="$out"
args=(--demo)
[ "${DSPI_SMOKE_HW:-}" = 1 ] && args=()
if [ "${DSPI_SMOKE_HW:-}" = replug ]; then
    # Each wait in the driver is 45 s, so this bounds a run with a slow owner
    timeout 330 "$app" 2>&1 | tee "$out/log.txt" | grep --line-buffered -E '^(smoke:|FAIL)'
    code=${PIPESTATUS[0]}
else
    timeout 300 "$app" "${args[@]}" > "$out/log.txt" 2>&1
    code=$?
    grep -E '^(smoke:|FAIL)' "$out/log.txt"
fi
if [ "$code" != 0 ]; then
    echo "DSPiConsoleSmoke exited with $code; the end of its log:" >&2
    tail -30 "$out/log.txt" >&2
    exit 1
fi
# A run that stopped early has no summary, whatever its exit code.
grep -q '^smoke: [0-9]* checks' "$out/log.txt" || { echo "no summary line: the driver did not finish" >&2; exit 1; }
# This catches the core's own warnings only. dspi-capi's limiter turns a message
# repeated after 30 s into "... (and N more)", a different line each time, and
# holds the rest back, so a dspi-capi failure polled through a run never counts
# here: its evidence is the N in the log.
noisy=$(grep -E '^\[.*WARN' "$out/log.txt" | sed -E 's/^\[[^ ]* *//' | sort | uniq -c | sort -rn | awk '$1 > 5')
if [ -n "$noisy" ]; then
    echo "repeated core warnings:" >&2
    echo "$noisy" >&2
    # A replug retries its reopen, so there a repeat is for reading, not a failure
    [ "${DSPI_SMOKE_HW:-}" = replug ] || exit 1
fi
# -- WP QD12 -- Whole launches with the tray settings on, each ending in a real
# quit: starting hidden is decided before main.qml loads, and Qt's own quit path
# only runs once. The driver fails by exit code, 3 if the app did not quit.
if [ -z "${DSPI_SMOKE_HW:-}" ]; then
    for mode in hidden hidden-quit app-quit no-tray; do
        printf '[onboarding]\nsetupDone=true\n[tray]\nshowInTray=true\nstartMinimised=true\n' > "$HOME/.config/DSPi/DSPi Console.conf"
        probe=1; [ "$mode" = no-tray ] && probe=0
        DSPI_SMOKE_TRAY=$mode DSPI_SMOKE_TRAY_PROBE=$probe timeout 60 "$app" --demo > "$out/log-tray-$mode.txt" 2>&1
        code=$?
        grep -E '^(smoke:|FAIL)' "$out/log-tray-$mode.txt"
        if [ "$code" != 0 ]; then
            echo "tray launch $mode exited with $code; the end of its log:" >&2
            tail -30 "$out/log-tray-$mode.txt" >&2
            exit 1
        fi
    done
fi
