#!/bin/bash
# Run DSPiConsoleSmoke offscreen on the demo device (ci/check.sh --step
# qt-offscreen). Passes when the driver reports no failure and the core logged
# no warning more than a few times over the run (a polled call that fails is
# reported once, not at the poll rate).
#
#   run.sh <DSPiConsoleSmoke> <output dir: log.txt and one PNG per window>
#
# DSPI_SMOKE_SECTION=<name>[,<name>...] runs only those sections of the driver
# (SmokeDriver.h). -- WP QP0 -- The hardware (DSPI_SMOKE_HW, run-hw.sh,
# run-replug.sh), demo-entry, tray and pseudo-locale passes come back with the
# work packages that rebuild their screens (QP19, QP14, QP18); until then the
# driver exits 2 when asked for one.
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
# -- WP QD7 -- This run starts with nothing connected and the demo not on offer yet
[ -n "${DSPI_SMOKE_DEMO_ENTRY:-}" ] && args=()
if [ "${DSPI_SMOKE_HW:-}" = replug ]; then
    # Each wait in the driver is 45 s, so this bounds a run with a slow owner
    timeout 330 "$app" 2>&1 | tee "$out/log.txt" | grep --line-buffered -E '^(smoke:|FAIL)'
    code=${PIPESTATUS[0]}
else
    # -- WP QD38 -- The driver's watchdog ends a run whose section hangs (exit 4,
    # naming the section; DSPI_SMOKE_SECTION_LIMIT, 120 s by default) and logs
    # each section's time. This total only backs it up: about twice the main
    # run's cost on a quiet host (285 s at gate 2), so a loaded host fits.
    # -- WP QD39 -- Rule kept, total raised: the main run reached 355 s quiet
    # and 386 s loaded after QD22 (review c6d), so twice quiet is 710 s; 900 s
    # leaves room for the next section. A hang is still caught first by the
    # section watchdog, and by this total at worst.
    timeout 900 "$app" "${args[@]}" > "$out/log.txt" 2>&1
    code=$?
    grep -E '^(smoke:|FAIL)' "$out/log.txt"
fi
if [ "$code" != 0 ]; then
    echo "DSPiConsoleSmoke exited with $code; the end of its log:" >&2
    tail -30 "$out/log.txt" >&2
    exit 1
fi
# -- WP QP0 -- --version names the app by its display name (AppIdentity.h), not
# by the settings name
name=$(sed -n 's/.*displayName = "\(.*\)";/\1/p' "$(dirname "$0")/../src/AppIdentity.h")
version=$("$app" --version 2>/dev/null)
case "$version" in
    "$name "[0-9]*) echo "smoke: --version: $version" ;;
    *) echo "FAIL --version printed '$version', not '$name <version>'" >&2; exit 1 ;;
esac
# A run that stopped early has no summary, whatever its exit code.
grep -q '^smoke: [0-9]* checks' "$out/log.txt" || { echo "no summary line: the driver did not finish" >&2; exit 1; }
# This catches the core's own warnings only. dspi-capi's limiter turns a message
# repeated after 30 s into "... (and N more)", a different line each time, and
# holds the rest back, so a dspi-capi failure polled through a run never counts
# here: its evidence is the N in the log.
# -- WP QD11 -- The guard logs each accepted typed answer on purpose, and the updater's
# tests answer many times over: that is not a call failing at the poll rate.
noisy=$(grep -E '^\[.*WARN' "$out/log.txt" | grep -vE '^\[WARN  dspi_core::firmware_guard\] firmware guard: risk accepted by typed confirmation: ' | sed -E 's/^\[[^ ]* *//' | sort | uniq -c | sort -rn | awk '$1 > 5')
if [ -n "$noisy" ]; then
    echo "repeated core warnings:" >&2
    echo "$noisy" >&2
    # A replug retries its reopen, so there a repeat is for reading, not a failure
    [ "${DSPI_SMOKE_HW:-}" = replug ] || exit 1
fi
