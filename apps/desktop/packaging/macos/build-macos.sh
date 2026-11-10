#!/bin/bash
# macOS release: a universal (x86_64 + arm64) "DSPi uConsole.app", made
# self-contained with macdeployqt, ad-hoc signed, and a .dmg of it.
#
#   QTDIR=~/Qt/6.11.2/macos apps/desktop/packaging/macos/build-macos.sh [output dir, default dist/]
#
# Needs Xcode (or its command line tools), rustup with the
# x86_64-apple-darwin and aarch64-apple-darwin targets, CMake, Ninja, and
# Qt 6 for macOS from qt.io (fetch-qt.sh, the online installer or
# aqtinstall; its "macos" build is universal). ARCHS="x86_64" or "arm64"
# builds one only. Set SIGN_ID to a "Developer ID Application: …" identity
# to sign for distribution (notarisation is a separate step); the default
# "-" is ad hoc. Qt stays dynamic (LGPL-3): its frameworks are copied into
# the bundle, and the build fails if the app does not load QtCore from there.
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)          # apps/desktop
repo=$(cd "$root/../.." && pwd)                      # the dspi-unified-console root
out=${1:-$repo/dist}
: "${QTDIR:?set QTDIR to the macos directory of Qt, e.g. ~/Qt/6.11.2/macos}"
archs=${ARCHS:-x86_64 arm64}
build=${BUILD_DIR:-$root/qt/build-macos}
target=${CARGO_TARGET_DIR:-$repo/target}
export MACOSX_DEPLOYMENT_TARGET=${MACOSX_DEPLOYMENT_TARGET:-13.0}

version=$(sed -n 's/.*setApplicationVersion("\([^"]*\)").*/\1/p' "$root/qt/src/main.cpp")
name="DSPi-uConsole-$version-macos"

# The core for each architecture, then one universal static library
libs=()
for a in $archs; do
    t=$a-apple-darwin; [ "$a" = arm64 ] && t=aarch64-apple-darwin
    (cd "$repo" && cargo build --release --locked -p dspi-capi --features link,midir,mdns,hub --target "$t")
    libs+=("$target/$t/release/libdspi_capi.a")
done
mkdir -p "$target/universal-apple-darwin/release"
lipo -create "${libs[@]}" -output "$target/universal-apple-darwin/release/libdspi_capi.a"

cmake -S "$root/qt" -B "$build" -G Ninja -DCMAKE_BUILD_TYPE=Release \
      -DCMAKE_PREFIX_PATH="$QTDIR" -DDSPI_QT_MAJOR=6 \
      -DCMAKE_OSX_ARCHITECTURES="${archs// /;}" -DCMAKE_OSX_DEPLOYMENT_TARGET="$MACOSX_DEPLOYMENT_TARGET" \
      -DDSPI_CARGO_TARGET_DIR="$target" -DDSPI_RUST_TARGET=universal-apple-darwin
cmake --build "$build"

mkdir -p "$out"
app="$out/DSPi uConsole.app"
rm -rf "$app"; cp -R "$build/DSPi uConsole.app" "$app"
"$QTDIR/bin/macdeployqt" "$app" -qmldir="$root/qt/qml" -always-overwrite
# SQL drivers come with QtQuick.LocalStorage; the app uses none, and these
# three link client libraries a Mac doesn't have (macdeployqt's "ERROR: no
# file at" lines for libmimerapi, libiodbc and libpq are about them)
rm -f "$app"/Contents/PlugIns/sqldrivers/libqsql{mimer,odbc,psql}.dylib

# -- WP QD21b -- Qt dynamic, from the bundle: the LGPL lets users swap it
exe="$app/Contents/MacOS/DSPi uConsole"
otool -L "$exe" | grep -q '@rpath/QtCore.framework/Versions/A/QtCore' \
    || { echo "build-macos: the app does not link QtCore dynamically" >&2; exit 1; }
[ -f "$app/Contents/Frameworks/QtCore.framework/Versions/A/QtCore" ] \
    || { echo "build-macos: QtCore.framework is not in the bundle" >&2; exit 1; }
if otool -l "$exe" | grep -A2 LC_RPATH | grep -q "$QTDIR"; then
    echo "build-macos: the app still looks for Qt in $QTDIR" >&2; exit 1
fi

# -- WP QD42 -- The licence texts, the notice and the list of bundled libraries
# (before signing: the signature covers them)
QTDIR=$QTDIR "$root/packaging/licences/stage.sh" macos "$app" "$app/Contents/Resources" "$version"

# The hardened runtime (needed for notarisation) only with a real identity:
# ad hoc, its library validation would refuse the bundled Qt plugins
if [ -n "${SIGN_ID:-}" ]; then
    codesign --force --deep --options runtime --timestamp --sign "$SIGN_ID" "$app"
else
    codesign --force --deep --sign - "$app"
fi
codesign --verify --deep --strict "$app"

rm -f "$out/$name.dmg"
hdiutil create -quiet -volname "DSPi uConsole" -srcfolder "$app" -format UDZO "$out/$name.dmg"
lipo -info "$exe"
ls -la "$out/$name.dmg"
