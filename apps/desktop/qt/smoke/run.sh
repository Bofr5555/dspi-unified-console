#!/bin/bash
# Run DSPiConsoleSmoke offscreen on the demo device (ci/check.sh --step
# qt-offscreen). Passes when the driver reports no failure and the core logged
# no warning more than a few times over the run (a polled call that fails is
# reported once, not at the poll rate).
#
#   run.sh <DSPiConsoleSmoke> <output dir: log.txt and one PNG per window>
set -uo pipefail
app=$1; out=$2
rm -rf "$out"; mkdir -p "$out/home"
export HOME="$out/home" XDG_CONFIG_HOME="$out/home/.config"
mkdir -p "$HOME/.config/DSPi"
printf '[onboarding]\nsetupDone=true\n' > "$HOME/.config/DSPi/DSPi Console.conf"
export QT_QPA_PLATFORM=offscreen QT_QUICK_BACKEND=software QT_FORCE_STDERR_LOGGING=1
export RUST_LOG=warn DSPI_SMOKE_OUT="$out"
timeout 300 "$app" --demo > "$out/log.txt" 2>&1
code=$?
grep -E '^(smoke:|FAIL)' "$out/log.txt"
if [ "$code" != 0 ]; then
    echo "DSPiConsoleSmoke exited with $code; the end of its log:" >&2
    tail -30 "$out/log.txt" >&2
    exit 1
fi
noisy=$(grep -E '^\[.*WARN' "$out/log.txt" | sed -E 's/^\[[^ ]* *//' | sort | uniq -c | sort -rn | awk '$1 > 5')
if [ -n "$noisy" ]; then
    echo "repeated core warnings:" >&2
    echo "$noisy" >&2
    exit 1
fi
