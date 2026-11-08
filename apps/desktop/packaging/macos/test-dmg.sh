#!/bin/bash
# Mounts the .dmg from build-macos.sh read-only and starts the app from it on
# the demo device for 15 s (DMG_RUN_SECONDS). Passes when the app is still
# running at the end and its log shows no QML module or component, Qt plugin,
# library or image that failed to load, and the bundle carries the licence texts
# (Contents/Resources/NOTICE.txt and LICENSES/, WP QD42). Needs the GUI session (from ssh:
# sudo launchctl asuser "$(id -u)" sudo -u "$USER" test-dmg.sh <dmg>).
#
#   apps/desktop/packaging/macos/test-dmg.sh <dmg>
set -uo pipefail
dmg=${1:?usage: test-dmg.sh <dmg>}
secs=${DMG_RUN_SECONDS:-15}
work=$(dirname "$dmg")
mnt=$work/mnt
log=$work/app.log

mkdir -p "$mnt"
hdiutil attach -quiet -readonly -nobrowse -mountpoint "$mnt" "$dmg" || { echo "test-dmg: cannot mount $dmg"; exit 1; }
detach() { hdiutil detach -quiet "$mnt" || hdiutil detach -quiet -force "$mnt"; }
trap detach EXIT

app="$mnt/DSPi Console.app"
codesign --verify --deep --strict "$app" || { echo "test-dmg: the signature does not verify"; exit 1; }
# -- WP QD42 -- the licence files
for f in NOTICE.txt LICENSES/GPL-3.0.txt LICENSES/LGPL-3.0.txt LICENSES/BUNDLED.txt LICENSES/Qt/THIRD-PARTY.txt LICENSES/Qt/LGPL-3.0-only.txt \
         LICENSES/TERMS.txt LICENSES/rust-crates.txt; do   # -- WP LIC1 -- TERMS.txt, rust-crates.txt
    [ -s "$app/Contents/Resources/$f" ] || { echo "test-dmg: no Contents/Resources/$f in the app"; exit 1; }
done
grep -q "^Qt	.*	qtbase-everywhere-src-" "$app/Contents/Resources/LICENSES/BUNDLED.txt" \
    || { echo "test-dmg: BUNDLED.txt does not name Qt's source archives"; exit 1; }
grep -q "any later version" "$app/Contents/Resources/NOTICE.txt" || { echo "test-dmg: NOTICE.txt does not say GPL-3.0-or-later"; exit 1; }
grep -q "No trademark rights (7e)" "$app/Contents/Resources/NOTICE.txt" || { echo "test-dmg: NOTICE.txt lacks the additional terms"; exit 1; }   # -- WP LIC1 --
echo "test-dmg: NOTICE.txt and LICENSES/ are in the app"
QT_FORCE_STDERR_LOGGING=1 "$app/Contents/MacOS/DSPi Console" --demo >"$log" 2>&1 &
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
