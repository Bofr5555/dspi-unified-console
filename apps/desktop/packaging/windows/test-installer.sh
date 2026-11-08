#!/bin/bash
# Tests the installer and uninstaller under Wine, silently (/S), in a fresh
# Wine prefix: they install and remove the app, and refuse (exit code 2,
# nothing replaced or removed) while the installed DSPi Console is running.
# Runs in the build image, after build-windows.sh:
#   ci/run.sh 'apps/desktop/packaging/windows/test-installer.sh /work/dist-windows/DSPi-Console-<v>-windows-x86_64-setup.exe'
# The app runs on the demo device under Xvfb; no device, no network needed.
set -uo pipefail
setup=$(realpath "$1")
export WINEPREFIX=${WINEPREFIX:-$HOME/wine-installer-test} WINEDEBUG=-all WINEDLLOVERRIDES="mscoree,mshtml="
# No spaces: NSIS reads /D= and _?= unquoted, and Wine quotes an argument with a space
dir='C:\DSPiTest'
unix="$WINEPREFIX/drive_c/DSPiTest"
fails=0
check() {  # <what> <expected exit code> <actual>
    if [ "$2" = "$3" ]; then echo "test-installer: ok: $1 (exit $3)"
    else echo "test-installer: FAIL: $1: exit $3, expected $2"; fails=$((fails + 1)); fi
}

rm -rf "$WINEPREFIX"
Xvfb :91 -screen 0 1280x800x24 -nolisten tcp >/dev/null 2>&1 &
xvfb=$!
export DISPLAY=:91
cleanup() { wineserver -k 2>/dev/null; kill "$xvfb" 2>/dev/null; wait 2>/dev/null; rm -rf "$WINEPREFIX"; }
trap cleanup EXIT
wineboot -i >/dev/null 2>&1; wineserver -w

wine "$setup" /S "/D=$dir"; check "install" 0 $?
[ -f "$unix/DSPiConsole.exe" ] || { echo "test-installer: FAIL: no DSPiConsole.exe after the install"; exit 1; }
# -- WP QD21c -- the publisher Apps & features shows
publisher=$(wine reg query 'HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\DSPiConsole' /v Publisher 2>/dev/null | tr -d '\r' | sed -n 's/.*REG_SZ *//p')
[ -n "$publisher" ] && echo "test-installer: ok: Publisher \"$publisher\"" \
    || { echo "test-installer: FAIL: no Publisher in the uninstall entry"; fails=$((fails + 1)); }

# The installed app, running (on the demo device). Only whether it stays up:
# under Wine the GUI exe's Qt messages reach no log this script can read
wine "$unix/DSPiConsole.exe" --demo >/dev/null 2>&1 &
app=$!
sleep 15
kill -0 "$app" 2>/dev/null || { echo "test-installer: FAIL: the app did not stay up under Wine"; exit 1; }
before=$(stat -c %Y "$unix/DSPiConsole.exe")
wine "$setup" /S "/D=$dir"; check "install while the app runs is refused" 2 $?
wine "$unix/Uninstall.exe" /S "_?=$dir"; check "uninstall while the app runs is refused" 2 $?
[ -f "$unix/DSPiConsole.exe" ] && [ "$(stat -c %Y "$unix/DSPiConsole.exe")" = "$before" ] \
    && [ -d "$unix/qml" ] && echo "test-installer: ok: the running app's files are untouched" \
    || { echo "test-installer: FAIL: files changed while the app ran"; fails=$((fails + 1)); }

# Quit it: the installer and uninstaller go through
kill "$app" 2>/dev/null; wineserver -k; wineserver -w; wait "$app" 2>/dev/null
wine "$setup" /S "/D=$dir"; check "install after the app quit" 0 $?
wine "$unix/Uninstall.exe" /S "_?=$dir"; check "uninstall after the app quit" 0 $?
[ ! -e "$unix/DSPiConsole.exe" ] && echo "test-installer: ok: the app's files are removed" \
    || { echo "test-installer: FAIL: DSPiConsole.exe is still there after the uninstall"; fails=$((fails + 1)); }

[ "$fails" = 0 ] && echo "test-installer: PASS" || echo "test-installer: $fails failed"
[ "$fails" = 0 ]
