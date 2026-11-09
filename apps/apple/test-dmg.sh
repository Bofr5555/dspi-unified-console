#!/bin/bash
# -- WP REL013M -- Mounts the Swift app's .dmg from package-macos.sh read-only
# and starts the app from it on the demo device (-LocalTransport demo) for 20 s
# (DMG_RUN_SECONDS), as packaging/macos/test-dmg.sh does for the Qt .dmg.
# Passes when the signature verifies, the bundle has the unified console's
# identifier and version and carries the licence texts (Contents/Resources:
# NOTICE.txt and LICENSES/), the app is still running at the end with no
# connection to the update feed open, it wrote its settings under its own
# identifier and not Weeb Labs' DSPi Console's, and no crash report appeared.
# Needs the GUI session (from ssh:
# sudo launchctl asuser "$(id -u)" sudo -u "$USER" test-dmg.sh <dmg>).
#
#   apps/apple/test-dmg.sh <dmg>
set -uo pipefail
dmg=${1:?usage: test-dmg.sh <dmg>}
secs=${DMG_RUN_SECONDS:-20}
id=invalid.dspi-unified-console.DSPiConsole
work=$(cd "$(dirname "$dmg")" && pwd)
mnt=$work/mnt-swift
log=$work/swift-app.log
fail() { echo "test-dmg: FAILED: $*"; exit 1; }

version=$(basename "$dmg" | sed -n 's/^DSPi-uConsole-Swift-\(.*\)-macos\.dmg$/\1/p')
[ -n "$version" ] || fail "$dmg is not named DSPi-uConsole-Swift-<version>-macos.dmg"
mkdir -p "$mnt"
hdiutil attach -quiet -readonly -nobrowse -mountpoint "$mnt" "$dmg" || fail "cannot mount $dmg"
detach() { hdiutil detach -quiet "$mnt" || hdiutil detach -quiet -force "$mnt"; }
trap detach EXIT

app="$mnt/DSPi uConsole.app"
codesign --verify --deep --strict "$app" || fail "the signature does not verify"
sig=$(codesign -dv "$app" 2>&1)
grep -q "Signature=adhoc" <<<"$sig" || fail "not ad-hoc signed"
plist="$app/Contents/Info.plist"
info() { /usr/libexec/PlistBuddy -c "Print :$1" "$plist" 2>/dev/null; }
[ "$(info CFBundleIdentifier)" = "$id" ] || fail "CFBundleIdentifier is $(info CFBundleIdentifier)"
[ "$(info CFBundleShortVersionString)" = "$version" ] || fail "CFBundleShortVersionString is $(info CFBundleShortVersionString)"
[ "$(info CFBundleVersion)" = "$(echo "$version" | cut -d. -f2)" ] || fail "CFBundleVersion is $(info CFBundleVersion)"
[ "$(info CFBundleName)" = "DSPi uConsole" ] || fail "CFBundleName is $(info CFBundleName)"
[ "$(info CFBundleExecutable)" = "DSPi uConsole" ] || fail "CFBundleExecutable is $(info CFBundleExecutable)"
echo "test-dmg: $id $(info CFBundleShortVersionString) ($(info CFBundleVersion)), name $(info CFBundleName), firmware $(info DSPiFirmwareVersion)"
lipo -info "$app/Contents/MacOS/DSPi uConsole"

r=$app/Contents/Resources
for f in NOTICE.txt LICENSES/GPL-3.0.txt LICENSES/TERMS.txt LICENSES/rust-crates.txt LICENSES/BUNDLED.txt \
         LICENSES/swift/swift-nio/LICENSE.txt; do
    [ -s "$r/$f" ] || fail "no Contents/Resources/$f in the app"
done
grep -q "any later version" "$r/NOTICE.txt" || fail "NOTICE.txt does not say GPL-3.0-or-later"
grep -q "No trademark rights (7e)" "$r/NOTICE.txt" || fail "NOTICE.txt lacks the additional terms"
grep -q "This program uses Qt" "$r/NOTICE.txt" && fail "NOTICE.txt names Qt as bundled"
grep -q "(dspi_ffi) in the Swift macOS app" "$r/LICENSES/rust-crates.txt" || fail "rust-crates.txt is not the Swift app's"
echo "test-dmg: NOTICE.txt and LICENSES/ are in the app"

prefs=$HOME/Library/Preferences
weeb_before=$(stat -f %m "$prefs/WeebLabs.DSPi-Console.plist" 2>/dev/null || echo none)
marker=$work/swift-start.marker
touch "$marker"
"$app/Contents/MacOS/DSPi uConsole" -LocalTransport demo >"$log" 2>&1 &
pid=$!
sleep "$secs"
net=$(lsof -nP -a -p "$pid" -i 2>/dev/null | sed 1d)
if kill -0 "$pid" 2>/dev/null; then
    alive=1; kill "$pid"; wait "$pid" 2>/dev/null
else
    alive=""; wait "$pid"; echo "test-dmg: the app exited with $? before $secs s"
fi
[ -n "$alive" ] || { cat "$log"; fail "the app did not keep running"; }
echo "test-dmg: the app from the .dmg ran $secs s on the demo device"
echo "test-dmg: its sockets at the end: ${net:-none}"
grep -E ':443 ' <<<"$net" && fail "a connection to port 443 was open (the update feed?)"
defaults read "$id" >/dev/null 2>&1 || fail "no settings under $id"
weeb_after=$(stat -f %m "$prefs/WeebLabs.DSPi-Console.plist" 2>/dev/null || echo none)
[ "$weeb_before" = "$weeb_after" ] || fail "WeebLabs.DSPi-Console.plist changed"
echo "test-dmg: settings under $id ($(defaults read "$id" 2>/dev/null | grep -c '=') keys); WeebLabs.DSPi-Console.plist: $weeb_after (unchanged)"
crashes=$(find "$HOME/Library/Logs/DiagnosticReports" /Library/Logs/DiagnosticReports -newer "$marker" -iname '*DSPi*' 2>/dev/null)
[ -z "$crashes" ] || fail "crash reports: $crashes"
echo "test-dmg: no crash reports"
echo "test-dmg: passed"
