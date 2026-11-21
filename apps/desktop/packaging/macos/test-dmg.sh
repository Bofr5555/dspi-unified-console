#!/bin/bash
# -- WP QTMAC -- Mounts the .dmg from make-dmg.sh read-only and checks it: the
# ad hoc signature, the Info.plist (names, bundle id, version, minimum macOS,
# the microphone and local network texts), both architectures, the licence
# files (Contents/Resources/NOTICE.txt and LICENSES/), --version; then starts
# the app from the .dmg on the demo device for 15 s (DMG_RUN_SECONDS). Passes
# when the app is still running at the end and its log shows no QML module or
# component, Qt plugin, library or image that failed to load. Needs the GUI
# session (from ssh: sudo launchctl asuser "$(id -u)" sudo -u "$USER" test-dmg.sh <dmg>).
#
#   apps/desktop/packaging/macos/test-dmg.sh <dmg>
set -uo pipefail
dmg=${1:?usage: test-dmg.sh <dmg>}
secs=${DMG_RUN_SECONDS:-15}
here=$(cd "$(dirname "$0")" && pwd)
work=$(cd "$(dirname "$dmg")" && pwd)/test
mnt=$work/mnt
log=$work/app.log
name=$(sed -n 's/.*displayName = "\(.*\)";/\1/p' "$here/../../qt/src/AppIdentity.h")
version=$(sed -n 's/^project(uconsole-for-dspi VERSION \([0-9.]*\).*/\1/p' "$here/../../qt/CMakeLists.txt")
fail() { echo "test-dmg: $*"; exit 1; }

rm -rf "$work"; mkdir -p "$mnt"
hdiutil attach -quiet -readonly -nobrowse -mountpoint "$mnt" "$dmg" || fail "cannot mount $dmg"
detach() { hdiutil detach -quiet "$mnt" || hdiutil detach -quiet -force "$mnt"; }
trap detach EXIT

app="$mnt/$name.app"
exe="$app/Contents/MacOS/$name"
[ -x "$exe" ] || fail "no $name.app in the .dmg"
[ -L "$mnt/Applications" ] || fail "no Applications link in the .dmg"
codesign --verify --deep --strict "$app" || fail "the signature does not verify"
sig=$(codesign -dv "$app" 2>&1 | sed -n 's/^Signature=//p')
echo "test-dmg: the signature verifies (${sig:-unknown})"
plist() { /usr/libexec/PlistBuddy -c "Print :$1" "$app/Contents/Info.plist" 2>/dev/null; }
for k in CFBundleName CFBundleDisplayName; do
    [ "$(plist $k)" = "$name" ] || fail "$k is \"$(plist $k)\", not $name"
done
[ "$(plist CFBundleIdentifier)" = se.bofr.uconsole-for-dspi ] || fail "CFBundleIdentifier is $(plist CFBundleIdentifier)"
[ "$(plist CFBundleShortVersionString)" = "$version" ] || fail "CFBundleShortVersionString is $(plist CFBundleShortVersionString), not $version"
min=$(plist LSMinimumSystemVersion)
minos=$(otool -l "$exe" | awk '/LC_BUILD_VERSION/{f=1} f && /minos/{print $2; exit}')
[ -n "$min" ] && [ "$min" = "$minos" ] || fail "LSMinimumSystemVersion $min, the binary's minos $minos"
for k in NSMicrophoneUsageDescription NSLocalNetworkUsageDescription NSRemovableVolumesUsageDescription; do
    [ -n "$(plist $k)" ] || fail "no $k"
done
[ "$(plist NSBonjourServices:0)" = _dspi._tcp ] || fail "NSBonjourServices does not name _dspi._tcp"
[ -s "$app/Contents/Resources/$(plist CFBundleIconFile).icns" ] || fail "no icon"
echo "test-dmg: Info.plist: $name $version, $(plist CFBundleIdentifier), macOS $min or later"
archs=$(lipo -archs "$exe")
case " $archs " in *" x86_64 "*) ;; *) fail "no x86_64 in $exe ($archs)" ;; esac
case " $archs " in *" arm64 "*) ;; *) fail "no arm64 in $exe ($archs)" ;; esac
echo "test-dmg: universal ($archs)"
for f in NOTICE.txt LICENSES/GPL-3.0.txt LICENSES/LGPL-3.0.txt LICENSES/BUNDLED.txt LICENSES/Qt/THIRD-PARTY.txt \
         LICENSES/Qt/LGPL-3.0-only.txt LICENSES/TERMS.txt LICENSES/rust-crates.txt LICENSES/Lucide/LICENSE \
         LICENSES/firmware-THIRD-PARTY-NOTICES.txt LICENSES/AutoEq-MIT.txt; do
    [ -s "$app/Contents/Resources/$f" ] || fail "no Contents/Resources/$f in the app"
done
grep -q "^Qt	.*	qtbase-everywhere-src-" "$app/Contents/Resources/LICENSES/BUNDLED.txt" \
    || fail "BUNDLED.txt does not name Qt's source archives"
grep -q "^Licence: GPL-3.0-only" "$app/Contents/Resources/NOTICE.txt" || fail "NOTICE.txt is not labelled GPL-3.0-only"
grep -q "No trademark rights (7e)" "$app/Contents/Resources/NOTICE.txt" || fail "NOTICE.txt lacks the additional terms"
grep -q "uConsole-for-DSPi-$version-desktop-source.tar.gz" "$app/Contents/Resources/NOTICE.txt" || fail "NOTICE.txt does not name the desktop source archive"
echo "test-dmg: NOTICE.txt and LICENSES/ are in the app"
said=$("$exe" --version 2>&1)
[ "$said" = "$name $version" ] || fail "--version says \"$said\""
echo "test-dmg: --version: $said"

QT_FORCE_STDERR_LOGGING=1 "$exe" --demo >"$log" 2>&1 &
pid=$!
sleep "$secs"
if kill -0 "$pid" 2>/dev/null; then
    alive=1; kill "$pid"; wait "$pid" 2>/dev/null
else
    alive=""; wait "$pid"; echo "test-dmg: the app exited with $? before $secs s"
fi
bad=$(grep -E "is not installed|failed to load component|is not a type|Could not load the Qt platform plugin|Library not loaded|Error decoding|Unsupported image format" "$log")
if [ -n "$alive" ] && [ -z "$bad" ]; then
    echo "test-dmg: the app from the .dmg ran $secs s on the demo device, no load errors"
else
    echo "test-dmg: FAILED; the log:"; cat "$log"; exit 1
fi
