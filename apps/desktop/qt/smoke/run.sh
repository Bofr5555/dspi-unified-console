#!/bin/bash
# Run uconsole-for-dspi-smoke offscreen on the demo device (ci/check.sh --step
# qt-offscreen). Passes when the driver reports no failure and the core logged
# no warning more than a few times over the run (a polled call that fails is
# reported once, not at the poll rate).
#
#   run.sh <uconsole-for-dspi-smoke> <output dir: log.txt and one PNG per window>
#
# DSPI_SMOKE_SECTION=<name>[,<name>...] runs only those sections of the driver
# (SmokeDriver.h).
# -- WP QP19 -- A whole run (behaviour.md §23) is the main run, then a launch
# each into its own directory: the demo chosen with nothing connected
# (demo-entry/), the four tray launches (tray-<launch>/), the pseudo-locale
# pair when the build has its section (pseudo-<locale>/), and the hardware pass
# on the demo (hw-dry/). DSPI_SMOKE_SECTION, or DSPI_SMOKE_HW set by run-hw.sh
# and run-replug.sh, runs that one launch only. Then <out>/index.html shows
# every PNG, the gallery section's dark and light pairs first.
# -- WP CIPAR -- run-shards.sh splits a whole run across several app instances:
# DSPI_SMOKE_MAIN=0 leaves out the main run (and the popups launches, which go
# with its qtpop-popups section), DSPI_SMOKE_LAUNCHES=1|0 runs the other
# launches or not (default: when no DSPI_SMOKE_SECTION or DSPI_SMOKE_HW names a
# part). And for the combined run:
#   run.sh --gallery <output dir>   writes <out>/index.html only
set -uo pipefail

# -- WP CIPAR -- gallery and backtraces moved up from below, unchanged, for
# --gallery and for a run without the main run (DSPI_SMOKE_MAIN=0)
# -- WP QP19 -- The gallery: <out>/index.html
gallery() {
    local html="$out/index.html" f base other
    {
        printf '<!doctype html><meta charset="utf-8"><title>Smoke screenshots</title>\n'
        printf '<style>body{font:14px sans-serif;margin:16px;background:#f4f4f4;color:#111}'
        printf 'figure{display:inline-block;margin:6px;vertical-align:top}img{max-width:480px;border:1px solid #888}'
        printf 'figcaption{font-size:12px}h2{margin-top:24px}</style>\n'
        printf '<h1>Smoke screenshots</h1><p>%s</p>\n' "$(grep -m1 '^smoke: [0-9]* checks' "$out/log.txt")"
        printf '<h2>Every window, dark and light</h2>\n'
        for f in "$out"/gallery/*-dark.png; do
            [ -e "$f" ] || continue
            base=$(basename "$f" -dark.png)
            printf '<div><h3>%s</h3>' "$base"
            printf '<figure><img src="gallery/%s-dark.png" loading="lazy"><figcaption>dark</figcaption></figure>' "$base"
            printf '<figure><img src="gallery/%s-light.png" loading="lazy"><figcaption>light</figcaption></figure></div>\n' "$base"
        done
        printf '<h2>States, by section and launch</h2>\n'
        while IFS= read -r other; do
            printf '<figure><a href="%s"><img src="%s" loading="lazy"></a><figcaption>%s</figcaption></figure>\n' "$other" "$other" "$other"
        done < <(cd "$out" && find . -name '*.png' -not -path './gallery/*' -not -path './*/home/*' -not -path './home/*' | sed 's|^\./||' | sort)
    } > "$html"
    echo "smoke: gallery: $html, $(find "$out/gallery" -name '*.png' 2>/dev/null | wc -l) window screenshots"
}
# -- WP QP19 -- A thread warning or a crash's backtrace fails the run, even one
# whose sections passed (the driver counts the warnings; this catches a line
# printed after the summary, at shutdown)
backtraces() {
    if grep -qE '^(smoke: BACKTRACE|FAIL thread:)' "$1"; then
        echo "a thread warning or a backtrace in $1:" >&2
        grep -E -A12 '^(smoke: BACKTRACE|FAIL thread:)' "$1" | head -60 >&2
        return 1
    fi
}
case "${1:-}" in
    --gallery) out=$2; gallery; exit 0 ;;
esac

app=$1; out=$2
main=${DSPI_SMOKE_MAIN:-1}
launches=${DSPI_SMOKE_LAUNCHES:-}
if [ -z "$launches" ]; then
    launches=0
    [ -n "${DSPI_SMOKE_HW:-}" ] || [ -n "${DSPI_SMOKE_SECTION:-}" ] || launches=1
fi
# -- WP CIPAR end --
rm -rf "$out"; mkdir -p "$out/home"
export HOME="$out/home" XDG_CONFIG_HOME="$out/home/.config"
mkdir -p "$HOME/.config/uconsole-for-dspi"
printf '[onboarding]\nsetupDone=true\n' > "$HOME/.config/uconsole-for-dspi/uconsole-for-dspi.conf"
export QT_QPA_PLATFORM=offscreen QT_QUICK_BACKEND=software QT_FORCE_STDERR_LOGGING=1
export RUST_LOG=warn DSPI_SMOKE_OUT="$out"
args=(--demo)
[ "${DSPI_SMOKE_HW:-}" = 1 ] && args=()
# -- WP QD7 -- This run starts with nothing connected and the demo not on offer yet
[ -n "${DSPI_SMOKE_DEMO_ENTRY:-}" ] && args=()
# -- WP CIPAR -- the main run and its checks, unless DSPI_SMOKE_MAIN=0
if [ "$main" = 1 ]; then
if [ "${DSPI_SMOKE_HW:-}" = replug ]; then
    # Each wait in the driver is 45 s, so this bounds a run with a slow owner
    DSPI_SMOKE_SECTION_LIMIT=${DSPI_SMOKE_SECTION_LIMIT:-330} timeout 330 "$app" 2>&1 | tee "$out/log.txt" | grep --line-buffered -E '^(smoke:|FAIL)'
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
    # -- review AT-E -- Rule kept, total raised again: the main run is 592 s
    # quiet after WP ATQTSIM (atqtsim-units 77 s, p5demo-sim 83 s), so twice
    # quiet is 1184 s; 900 s left a loaded host a third of the run as margin.
    # -- WP LAY7 -- Rule kept, total raised: 966 s quiet after wave 5, about
    # 1035 s with both layouts' galleries and accessibility passes.
    timeout 2400 "$app" "${args[@]}" > "$out/log.txt" 2>&1
    code=$?
    grep -E '^(smoke:|FAIL)' "$out/log.txt"
fi
if [ "$code" != 0 ]; then
    echo "uconsole-for-dspi-smoke exited with $code; the end of its log:" >&2
    tail -30 "$out/log.txt" >&2
    exit 1
fi
# -- WP QP19 -- A thread warning or a crash's backtrace fails the run (backtraces, above)
backtraces "$out/log.txt" || exit 1
# -- WP QP0 -- --version names the app by its display name (AppIdentity.h), not
# by the settings name
name=$(sed -n 's/.*displayName = "\(.*\)";/\1/p' "$(dirname "$0")/../src/AppIdentity.h")
# -- WP QTCLI -- with no display at all (no offscreen platform, no X11 or
# Wayland): --version and --help are answered before the GUI is made
nodisplay=(env -u QT_QPA_PLATFORM -u DISPLAY -u WAYLAND_DISPLAY)
version=$("${nodisplay[@]}" "$app" --version 2>/dev/null); code=$?
case "$code:$version" in
    "0:$name "[0-9]*) echo "smoke: --version (no display): $version" ;;
    *) echo "FAIL --version exited $code, printed '$version', not '$name <version>'" >&2; exit 1 ;;
esac
help=$("${nodisplay[@]}" "$app" --help 2>/dev/null); code=$?
case "$code:$help" in
    0:*--version*--demo*) echo "smoke: --help (no display): exit 0" ;;
    *) echo "FAIL --help exited $code without a display; it printed: $help" >&2; exit 1 ;;
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

# -- WP LAY3b -- The table of actions the web and the Terminal read is the app's
# (section lay3b-actions; tools/actions.sh writes it)
if [ -f "$out/actions.json" ] && ! diff -u "$(dirname "$0")/../../../../assets/actions.json" "$out/actions.json"; then
    echo "FAIL assets/actions.json is not MenuTable's: run ci/run.sh 'apps/desktop/qt/tools/actions.sh'" >&2
    exit 1
fi
fi  # -- WP CIPAR -- main

# -- WP QP19 -- The other launches, each with its own home and settings:
#   launch <dir> <settings lines> [VAR=value...] -- [app arguments...]
# Its exit code is the app's, after the log checks of the main run.
launch() {
    local dir=$1 settings=$2; shift 2
    local vars=()
    while [ "$1" != -- ]; do vars+=("$1"); shift; done
    shift
    local o="$out/$dir"
    mkdir -p "$o/home/.config/uconsole-for-dspi"
    printf '[onboarding]\nsetupDone=true\n%b' "$settings" > "$o/home/.config/uconsole-for-dspi/uconsole-for-dspi.conf"
    env HOME="$o/home" XDG_CONFIG_HOME="$o/home/.config" DSPI_SMOKE_OUT="$o" "${vars[@]}" timeout 300 "$app" "$@" > "$o/log.txt" 2>&1
    local c=$?
    grep -E '^(smoke:|FAIL)' "$o/log.txt" | sed "s|^|$dir: |"
    [ "$c" = 0 ] || return "$c"
    backtraces "$o/log.txt" || return 1
    grep -q '^smoke: [0-9]* checks' "$o/log.txt" || { echo "FAIL $dir: no summary line" >&2; return 1; }
}
failed() {
    echo "FAIL $1: exited with $2; the end of its log:" >&2
    tail -30 "$out/$1/log.txt" >&2
    extra=1
}
extra=0
if [ "$launches" = 1 ]; then  # -- WP CIPAR -- was: no DSPI_SMOKE_HW and no DSPI_SMOKE_SECTION
    # QD7: nothing connected and the demo not open
    launch demo-entry '' DSPI_SMOKE_DEMO_ENTRY=1 -- || failed demo-entry $?
    # -- WP LAY7 -- and in Modern, with what else starts in it
    modern='[ui]\nlayout=modern\n'
    launch demo-entry-modern "$modern" DSPI_SMOKE_DEMO_ENTRY=modern -- || failed demo-entry-modern $?
    launch tray-hidden-modern "[tray]\nshowInTray=true\nstartMinimised=true\n$modern" DSPI_SMOKE_TRAY=hidden DSPI_SMOKE_TRAY_PROBE=1 -- --demo || failed tray-hidden-modern $?
    launch hw-dry-modern "$modern" DSPI_SMOKE_HW=dry -- --demo || failed hw-dry-modern $?
    # §12.1: the tray settings on; the stand-in says whether the desktop has a tray.
    # A launch that does not quit by itself exits 3.
    tray_on='[tray]\nshowInTray=true\nstartMinimised=true\n'
    tray_shown='[tray]\nshowInTray=true\nstartMinimised=false\n'
    launch tray-hidden "$tray_on" DSPI_SMOKE_TRAY=hidden DSPI_SMOKE_TRAY_PROBE=1 -- --demo || failed tray-hidden $?
    launch tray-hidden-quit "$tray_on" DSPI_SMOKE_TRAY=hidden-quit DSPI_SMOKE_TRAY_PROBE=1 -- --demo || failed tray-hidden-quit $?
    launch tray-app-quit "$tray_shown" DSPI_SMOKE_TRAY=app-quit DSPI_SMOKE_TRAY_PROBE=1 -- --demo || failed tray-app-quit $?
    launch tray-no-tray "$tray_on" DSPI_SMOKE_TRAY=no-tray DSPI_SMOKE_TRAY_PROBE=0 -- --demo || failed tray-no-tray $?
    # The pseudo-locale pair, once the build has its section (QP18); exit 2 until then
    for locale in en pseudo; do
        launch "pseudo-$locale" '' DSPI_SMOKE_PSEUDO=$locale -- --demo
        c=$?
        if [ "$c" = 2 ] && grep -q 'has no section in this build' "$out/pseudo-$locale/log.txt"; then
            rm -rf "$out/pseudo-$locale"
        elif [ "$c" != 0 ]; then
            failed "pseudo-$locale" "$c"
        fi
    done
    # The hardware pass on the demo
    launch hw-dry '' DSPI_SMOKE_HW=dry -- --demo || failed hw-dry $?
fi
# -- WP QTPOP -- The tool windows' sizes on two common screens: the offscreen
# platform's screen from a config file. Also run by DSPI_SMOKE_SECTION=qtpop-popups.
# -- WP CIPAR -- with the main run only
if [ "$main" = 1 ] && [ -z "${DSPI_SMOKE_HW:-}" ] && { [ -z "${DSPI_SMOKE_SECTION:-}" ] || [[ ",$DSPI_SMOKE_SECTION," == *,qtpop-popups,* ]]; }; then
    for screen in 1366x768 1920x1080; do
        printf '{"screens":[{"name":"smoke","x":0,"y":0,"width":%d,"height":%d,"logicalDpi":96,"logicalBaseDpi":96,"dpr":1}]}\n' \
            "${screen%x*}" "${screen#*x}" > "$out/screen-$screen.json"
        launch "popups-$screen" '' QT_QPA_PLATFORM="offscreen:configfile=$out/screen-$screen.json" \
            DSPI_SMOKE_POPUPS="$screen" DSPI_SMOKE_SECTION= -- --demo || failed "popups-$screen" $?
    done
fi

# -- WP QP19 -- The gallery: <out>/index.html (gallery, above)
gallery
[ "$extra" = 0 ] || exit 1
