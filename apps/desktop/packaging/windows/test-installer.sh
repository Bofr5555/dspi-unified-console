#!/bin/bash
# Tests the installer and uninstaller under Wine, silently (/S), in a fresh
# Wine prefix: they install and remove the app, and refuse (exit code 2,
# nothing replaced or removed) while the installed DSPi uConsole is running.
# Runs in the build image, after build-windows.sh:
#   ci/run.sh 'apps/desktop/packaging/windows/test-installer.sh /work/dist-windows/DSPi-uConsole-<v>-windows-x86_64-setup.exe'
# The app runs on the demo device under Xvfb; no device, no network needed.
# -- WP QD42 -- The install, and the zip beside the installer, carry the licence
# texts: NOTICE.txt and LICENSES/ (Qt's, GCC's runtime exception, every DLL
# listed in BUNDLED.txt).
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
key='HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\DSPiuConsole'
publisher=$(wine reg query "$key" /v Publisher 2>/dev/null | tr -d '\r' | sed -n 's/.*REG_SZ *//p')
[ -n "$publisher" ] && echo "test-installer: ok: Publisher \"$publisher\"" \
    || { echo "test-installer: FAIL: no Publisher in the uninstall entry"; fails=$((fails + 1)); }
# -- WP REL013R -- the name in Settings > Apps and the Start menu
display=$(wine reg query "$key" /v DisplayName 2>/dev/null | tr -d '\r' | sed -n 's/.*REG_SZ *//p')
menu=$(find "$WINEPREFIX/drive_c/users" -path '*Start Menu/Programs/*' -name '*.lnk' -printf '%f\n' 2>/dev/null)
[ "$display" = "DSPi uConsole" ] && [ "$menu" = "DSPi uConsole.lnk" ] \
    && echo "test-installer: ok: DisplayName \"$display\", Start menu \"$menu\"" \
    || { echo "test-installer: FAIL: DisplayName \"$display\", Start menu \"$menu\" (expected DSPi uConsole)"; fails=$((fails + 1)); }

# -- WP QD42 -- the licence files
lic_ok=1
for f in NOTICE.txt LICENSES/GPL-3.0.txt LICENSES/LGPL-3.0.txt LICENSES/BUNDLED.txt LICENSES/Qt/THIRD-PARTY.txt \
         LICENSES/Qt/LGPL-3.0-only.txt LICENSES/bundled/mingw64-libgcc/COPYING.RUNTIME LICENSES/bundled/mingw64-libstdc++/COPYING.RUNTIME \
         LICENSES/TERMS.txt LICENSES/rust-crates.txt; do
    [ -s "$unix/$f" ] || { echo "test-installer: FAIL: no $f in the install"; lic_ok=; }
done
grep -q "any later version" "$unix/NOTICE.txt" || { echo "test-installer: FAIL: NOTICE.txt does not say GPL-3.0-or-later"; lic_ok=; }
# -- WP LIC1 -- the additional terms in the notice, and the Rust crates notice
grep -q "section 7 (LICENSES/TERMS.txt" "$unix/NOTICE.txt" && grep -q "No trademark rights (7e)" "$unix/NOTICE.txt" \
    || { echo "test-installer: FAIL: NOTICE.txt lacks the additional terms"; lic_ok=; }
grep -q "^Third-party Rust crates linked into" "$unix/LICENSES/rust-crates.txt" 2>/dev/null \
    || { echo "test-installer: FAIL: LICENSES/rust-crates.txt is not the Rust crates notice"; lic_ok=; }
# Every bundled package has its licence text (a non-empty LICENSES/bundled/<package>/)
while IFS='	' read -r pkgname _; do
    case $pkgname in '#'*|'') continue ;; esac
    [ -n "$(find "$unix/LICENSES/bundled/$pkgname" -type f -size +0 2>/dev/null)" ] \
        || { echo "test-installer: FAIL: no licence file for $pkgname in LICENSES/bundled/"; lic_ok=; }
done <"$unix/LICENSES/BUNDLED.txt"
for dll in "$unix"/*.dll; do
    b=${dll##*/}
    [ "$b" = dspi_capi.dll ] && continue
    cut -f6 "$unix/LICENSES/BUNDLED.txt" | tr ' ' '\n' | grep -qx "$b" || { echo "test-installer: FAIL: $b is not in BUNDLED.txt"; lic_ok=; }
done
zip=${setup%-setup.exe}.zip
if [ -f "$zip" ]; then
    names=$(python3 -m zipfile -l "$zip" | awk '{print $1}')
    for f in NOTICE.txt LICENSES/BUNDLED.txt LICENSES/Qt/THIRD-PARTY.txt LICENSES/TERMS.txt LICENSES/rust-crates.txt; do
        grep -q "^[^/]*/$f$" <<<"$names" || { echo "test-installer: FAIL: no $f in ${zip##*/}"; lic_ok=; }
    done
fi
[ -n "$lic_ok" ] && echo "test-installer: ok: NOTICE.txt and LICENSES/ installed${zip:+ and in the zip}, every DLL listed, every bundled package with its licence file" \
    || fails=$((fails + 1))

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
