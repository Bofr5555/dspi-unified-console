#!/bin/bash
# Tests the installer and uninstaller under Wine, silently (/S), in a fresh
# Wine prefix: they install and remove the app, and refuse (exit code 2,
# nothing replaced or removed) while the installed app is running.
# Runs in the build image, after build-windows.sh:
#   ci/run.sh 'apps/desktop/packaging/windows/test-installer.sh /work/dist-windows/uConsole-for-DSPi-<v>-windows-x86_64-setup.exe'
# The app runs on the demo device under Xvfb; no device, no network needed.
# -- WP QD42 -- The install, and the zip beside the installer, carry the licence
# texts: NOTICE.txt and LICENSES/ (Qt's, GCC's runtime exception, every DLL
# listed in BUNDLED.txt).
# -- WP QP20 -- The name people see is "uConsole for DSPi"; the notice labels
# it GPL-3.0-only and carries Lucide's licence. -- WP RN1 -- Every entry the
# installer writes carries that name: the uninstall key, the key recording the
# folder, the Start menu shortcut, and the open-at-login value it removes.
# -- WP SWAPFIX -- The settings are an INI file under %APPDATA% (none in the
# registry); the uninstaller removes it and the app's firewall rule (netsh,
# stood in for by a program that logs its calls).
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

key='HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\uConsole for DSPi'
dirkey='HKCU\Software\uConsole for DSPi'

wine "$setup" /S "/D=$dir"; check "install" 0 $?
[ -f "$unix/uconsole-for-dspi.exe" ] || { echo "test-installer: FAIL: no uconsole-for-dspi.exe after the install"; exit 1; }
# -- WP QD21c -- the publisher Apps & features shows
publisher=$(wine reg query "$key" /v Publisher 2>/dev/null | tr -d '\r' | sed -n 's/.*REG_SZ *//p')
[ -n "$publisher" ] && echo "test-installer: ok: Publisher \"$publisher\"" \
    || { echo "test-installer: FAIL: no Publisher in the uninstall entry"; fails=$((fails + 1)); }
# -- WP REL013R -- the name in Settings > Apps and the Start menu
display=$(wine reg query "$key" /v DisplayName 2>/dev/null | tr -d '\r' | sed -n 's/.*REG_SZ *//p')
menu=$(find "$WINEPREFIX/drive_c/users" -path '*Start Menu/Programs/*' -name '*.lnk' -printf '%f\n' 2>/dev/null)
[ "$display" = "uConsole for DSPi" ] && [ "$menu" = "uConsole for DSPi.lnk" ] \
    && echo "test-installer: ok: DisplayName \"$display\", Start menu \"$menu\"" \
    || { echo "test-installer: FAIL: DisplayName \"$display\", Start menu \"$menu\" (expected uConsole for DSPi, one shortcut)"; fails=$((fails + 1)); }
# -- WP RN1 -- the folder recorded under the app's name, where an update finds it
recorded=$(wine reg query "$dirkey" /v InstallDir 2>/dev/null | tr -d '\r' | sed -n 's/.*REG_SZ *//p')
[ "$recorded" = "$dir" ] && echo "test-installer: ok: $dirkey InstallDir $recorded" \
    || { echo "test-installer: FAIL: $dirkey InstallDir is \"$recorded\", expected $dir"; fails=$((fails + 1)); }
version=$(wine reg query "$key" /v DisplayVersion 2>/dev/null | tr -d '\r' | sed -n 's/.*REG_SZ *//p')
case $setup in *"-$version-windows-x86_64-setup.exe") echo "test-installer: ok: DisplayVersion $version" ;;
    *) echo "test-installer: FAIL: DisplayVersion \"$version\" is not the setup's"; fails=$((fails + 1)) ;; esac

# -- WP QD42 -- the licence files
lic_ok=1
for f in NOTICE.txt LICENSES/GPL-3.0.txt LICENSES/LGPL-3.0.txt LICENSES/BUNDLED.txt LICENSES/Qt/THIRD-PARTY.txt \
         LICENSES/Qt/LGPL-3.0-only.txt LICENSES/bundled/mingw64-libgcc/COPYING.RUNTIME LICENSES/bundled/mingw64-libstdc++/COPYING.RUNTIME \
         LICENSES/TERMS.txt LICENSES/rust-crates.txt LICENSES/Lucide/LICENSE; do
    [ -s "$unix/$f" ] || { echo "test-installer: FAIL: no $f in the install"; lic_ok=; }
done
# -- WP QP20 -- the package's label, its name, our contributions' or-later, Lucide
flat=$(tr '\n' ' ' <"$unix/NOTICE.txt" | tr -s ' ')
for s in "Licence: GPL-3.0-only" "uConsole for DSPi" "any later version" "LICENSES/Lucide/LICENSE" "-desktop-source.tar.gz"; do
    case $flat in *"$s"*) ;; *) echo "test-installer: FAIL: NOTICE.txt does not say \"$s\""; lic_ok= ;; esac
done
grep -q "^ISC License" "$unix/LICENSES/Lucide/LICENSE" || { echo "test-installer: FAIL: LICENSES/Lucide/LICENSE is not Lucide's licence"; lic_ok=; }
# -- WP LIC1 -- the additional terms in the notice, and the Rust crates notice
case $flat in *"section 7 (LICENSES/TERMS.txt"*) ;; *) echo "test-installer: FAIL: NOTICE.txt lacks the additional terms"; lic_ok= ;; esac
grep -q "No trademark rights (7e)" "$unix/NOTICE.txt" || { echo "test-installer: FAIL: NOTICE.txt lacks the additional terms"; lic_ok=; }
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
    for f in NOTICE.txt LICENSES/BUNDLED.txt LICENSES/Qt/THIRD-PARTY.txt LICENSES/TERMS.txt LICENSES/rust-crates.txt LICENSES/Lucide/LICENSE; do
        grep -q "^[^/]*/$f$" <<<"$names" || { echo "test-installer: FAIL: no $f in ${zip##*/}"; lic_ok=; }
    done
fi
[ -n "$lic_ok" ] && echo "test-installer: ok: NOTICE.txt and LICENSES/ installed${zip:+ and in the zip}, every DLL listed, every bundled package with its licence file" \
    || fails=$((fails + 1))

# The installed app, running (on the demo device). Only whether it stays up:
# under Wine the GUI exe's Qt messages reach no log this script can read
wine "$unix/uconsole-for-dspi.exe" --demo >/dev/null 2>&1 &
app=$!
sleep 15
kill -0 "$app" 2>/dev/null || { echo "test-installer: FAIL: the app did not stay up under Wine"; exit 1; }
before=$(stat -c %Y "$unix/uconsole-for-dspi.exe")
wine "$setup" /S "/D=$dir"; check "install while the app runs is refused" 2 $?
wine "$unix/Uninstall.exe" /S "_?=$dir"; check "uninstall while the app runs is refused" 2 $?
[ -f "$unix/uconsole-for-dspi.exe" ] && [ "$(stat -c %Y "$unix/uconsole-for-dspi.exe")" = "$before" ] \
    && [ -d "$unix/qml" ] && echo "test-installer: ok: the running app's files are untouched" \
    || { echo "test-installer: FAIL: files changed while the app ran"; fails=$((fails + 1)); }

# Quit it: the installer and uninstaller go through
kill "$app" 2>/dev/null; wineserver -k; wineserver -w; wait "$app" 2>/dev/null
# -- WP SWAPFIX -- the settings are an INI file in the app's AppData folder,
# never the registry (owner 2026-11-13, behaviour.md §21): a new install
# writes onboarding/whatsNewShown at its first start
appdata=$(find "$WINEPREFIX/drive_c/users" -maxdepth 3 -path '*/AppData/Roaming' -type d 2>/dev/null | head -1)
ini="$appdata/uconsole-for-dspi/uconsole-for-dspi.ini"
grep -q '^whatsNewShown=' "$ini" 2>/dev/null && echo "test-installer: ok: the settings are ${ini#"$WINEPREFIX/"}" \
    || { echo "test-installer: FAIL: no settings in ${ini#"$WINEPREFIX/"}"; fails=$((fails + 1)); }
wine reg query 'HKCU\Software\uconsole-for-dspi' >/dev/null 2>&1 \
    && { echo "test-installer: FAIL: settings in the registry (HKCU\Software\uconsole-for-dspi)"; fails=$((fails + 1)); } \
    || echo "test-installer: ok: no settings in the registry"
# -- WP SWAPFIX -- a stand-in for netsh.exe that logs what it is asked and
# keeps one firewall rule as a file: `show rule` answers 0 while C:\netsh-rule
# exists (netsh's "No rules match" is exit 1), `delete rule` removes it
fake="$WINEPREFIX/drive_c/netsh-fake.c"
cat >"$fake" <<'C'
#include <stdio.h>
#include <string.h>
int main(int argc, char **argv) {
    FILE *log = fopen("C:\\netsh-log.txt", "a");
    for (int i = 1; i < argc; i++) fprintf(log, "%s%s", i > 1 ? " " : "", argv[i]);
    fprintf(log, "\n");
    fclose(log);
    if (argc > 3 && !strcmp(argv[3], "show")) {
        FILE *rule = fopen("C:\\netsh-rule", "r");
        if (!rule) { puts("No rules match the specified criteria."); return 1; }
        fclose(rule);
        return 0;
    }
    if (argc > 3 && !strcmp(argv[3], "delete")) return remove("C:\\netsh-rule") == 0 ? 0 : 1;
    return 0;
}
C
for d in system32 syswow64; do
    [ -d "$WINEPREFIX/drive_c/windows/$d" ] && x86_64-w64-mingw32-gcc -O2 -o "$WINEPREFIX/drive_c/windows/$d/netsh.exe" "$fake"
done
export WINEDLLOVERRIDES="$WINEDLLOVERRIDES;netsh.exe=n"
netshlog="$WINEPREFIX/drive_c/netsh-log.txt"
rule="$WINEPREFIX/drive_c/netsh-rule"
# -- WP QPI4 -- open at login on, as the app writes it: an update keeps it,
# the uninstall removes it
run='HKCU\Software\Microsoft\Windows\CurrentVersion\Run'
wine reg add "$run" /v "uConsole for DSPi" /d "\"$dir\\uconsole-for-dspi.exe\"" /f >/dev/null 2>&1
wine "$setup" /S; check "install after the app quit (an update: into the recorded folder)" 0 $?
wine reg query "$run" /v "uConsole for DSPi" >/dev/null 2>&1 && echo "test-installer: ok: an update keeps open at login" \
    || { echo "test-installer: FAIL: the update removed the open-at-login entry"; fails=$((fails + 1)); }
rm -f "$netshlog"; : >"$rule"   # -- WP SWAPFIX -- the app's rule is there
wine "$unix/Uninstall.exe" /S "_?=$dir"; check "uninstall after the app quit" 0 $?
# -- WP SWAPFIX -- the firewall rule and the settings file go with the app
# (the stand-in's log has Windows line ends)
tr -d '\r' <"$netshlog" 2>/dev/null | grep -qx 'advfirewall firewall delete rule name=uConsole for DSPi' && [ ! -e "$rule" ] \
    && echo "test-installer: ok: the app's firewall rule is removed" \
    || { echo "test-installer: FAIL: the firewall rule stays (netsh: $(tr '\n' '|' <"$netshlog" 2>/dev/null))"; fails=$((fails + 1)); }
[ ! -e "$ini" ] && echo "test-installer: ok: the settings file is removed" \
    || { echo "test-installer: FAIL: the settings file is still there"; fails=$((fails + 1)); }
wine reg query "$run" /v "uConsole for DSPi" >/dev/null 2>&1 \
    && { echo "test-installer: FAIL: the open-at-login entry is still there"; fails=$((fails + 1)); } \
    || echo "test-installer: ok: the open-at-login entry is removed"
[ ! -e "$unix/uconsole-for-dspi.exe" ] && echo "test-installer: ok: the app's files are removed" \
    || { echo "test-installer: FAIL: uconsole-for-dspi.exe is still there after the uninstall"; fails=$((fails + 1)); }
left=$(find "$WINEPREFIX/drive_c/users" -path '*Start Menu/Programs/*' -name '*.lnk' -printf '%f ' 2>/dev/null)
wine reg query "$key" >/dev/null 2>&1 && { echo "test-installer: FAIL: the uninstall key is still there"; fails=$((fails + 1)); }
wine reg query "$dirkey" >/dev/null 2>&1 && { echo "test-installer: FAIL: $dirkey is still there"; fails=$((fails + 1)); }
[ -z "$left" ] && echo "test-installer: ok: no Start menu shortcut, uninstall key or folder key left" \
    || { echo "test-installer: FAIL: Start menu shortcuts left: $left"; fails=$((fails + 1)); }

# -- WP SWAPFIX -- without a rule nothing is deleted (and nobody is asked)
wine "$setup" /S "/D=$dir"; check "install again" 0 $?
rm -f "$netshlog" "$rule"
wine "$unix/Uninstall.exe" /S "_?=$dir"; check "uninstall with no firewall rule" 0 $?
grep -q ' show rule ' "$netshlog" 2>/dev/null && ! grep -q ' delete rule ' "$netshlog" \
    && echo "test-installer: ok: no firewall rule, nothing deleted" \
    || { echo "test-installer: FAIL: netsh without a rule: $(tr '\n' '|' <"$netshlog" 2>/dev/null)"; fails=$((fails + 1)); }

[ "$fails" = 0 ] && echo "test-installer: PASS" || echo "test-installer: $fails failed"
[ "$fails" = 0 ]
