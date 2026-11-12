#!/bin/bash
# -- WP REL013M -- ci/macvm.sh swift-dmg, in the macOS VM inside the GUI
# session: the Swift app's .dmg with package-macos.sh into $CI_OUT/swift-dmg
# (default apps/apple/build/ci), then test-dmg.sh starts it from the mounted
# .dmg on the demo device. ci/macvm.sh copies the .dmg back.
#
#   apps/apple/ci-dmg.sh
set -euo pipefail
[ "$(uname -s)" = Darwin ] || { echo "ci-dmg: macOS only (the macOS VM)" >&2; exit 2; }
here=$(cd "$(dirname "$0")" && pwd)
out=${CI_OUT:-$here/build/ci}/swift-dmg
rm -rf "$out"
DERIVED_DATA=${DERIVED_DATA:-$out/../dd-swift-dmg} "$here/package-macos.sh" "$out"
"$here/test-dmg.sh" "$out"/uConsole-for-DSPi-*-macos.dmg
