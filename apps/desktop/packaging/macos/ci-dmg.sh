#!/bin/bash
# The build matrix's qt-dmg row (WP QD21b), in the macOS VM through
# ci/macvm.sh qt-dmg, which copies the tree there first and runs this inside
# the GUI session.
#
#   apps/desktop/packaging/macos/ci-dmg.sh
#
# Fetches Qt, CMake and Ninja from download.qt.io into $QT_ROOT (default
# ~/work/qt, verified, kept between runs), builds the .dmg with build-macos.sh
# into $CI_OUT/qt-dmg (default apps/desktop/qt/build-macos-ci), then starts
# the app from the mounted .dmg on the demo device (test-dmg.sh). The .dmg is
# unsigned (ad hoc) and is not published.
set -euo pipefail
[ "$(uname -s)" = Darwin ] || { echo "ci-dmg: macOS only (the macOS VM)" >&2; exit 2; }
here=$(cd "$(dirname "$0")" && pwd)
out=${CI_OUT:-$here/../../qt/build-macos-ci}/qt-dmg

tools=$("$here/fetch-qt.sh" "${QT_ROOT:-$HOME/work/qt}")
eval "$tools"
export QTDIR PATH="$CMAKE_DIR:$NINJA_DIR:$PATH"

rm -rf "$out"
BUILD_DIR=${BUILD_DIR:-$out/../build-qt-dmg} "$here/build-macos.sh" "$out"
"$here/test-dmg.sh" "$out"/DSPi-uConsole-*-macos.dmg
