#!/bin/bash
# -- WP QTMAC -- The macOS release of the Qt app, on a Mac (the macOS VM;
# build-macos.sh runs this there from a clean commit): a universal (x86_64 +
# arm64) "uConsole for DSPi.app", made self-contained with macdeployqt, ad hoc
# signed, not notarised, in uConsole-for-DSPi-<version>-macos.dmg.
#
#   apps/desktop/packaging/macos/make-dmg.sh <output dir>
#
# Needs Xcode (or its command line tools) and rustup with the
# x86_64-apple-darwin and aarch64-apple-darwin targets. Qt, CMake and Ninja
# come from fetch-qt.sh (the pins in qt-<version>.sha256, the Linux release's
# Qt) into $QT_PREFIX, default ~/qtmac-tools/qt-<version>. Other settings:
#   BUILD_DIR             CMake's build dir (emptied first), default <output dir>/build
#   CARGO_TARGET_DIR      cargo's, default <output dir>/target
#   DSPI_UPDATE_FEED_URL  the update checker's feed, as build-ubuntu.sh (empty: none)
#   SIGN_ID               a "Developer ID Application: ..." identity; default "-",
#                         ad hoc (no Developer ID exists yet: PRO1 E5)
# Qt stays dynamic (LGPL-3): its frameworks are copied into the bundle and the
# build fails unless the app loads them from there. As on Linux and Windows no
# Qt Multimedia plugin is bundled (the microphone's CoreAudio backend is in
# QtMultimedia itself), so neither FFmpeg nor AVFoundation playback is in the
# package; nor are Qt's SQL drivers.
set -euo pipefail
[ "$(uname -s)" = Darwin ] || { echo "make-dmg: macOS only" >&2; exit 2; }
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)        # apps/desktop
repo=$(cd "$root/../.." && pwd)
out=${1:?usage: make-dmg.sh <output dir>}
mkdir -p "$out"; out=$(cd "$out" && pwd)
manifest=$(ls "$here"/qt-*.sha256)
[ "$(echo "$manifest" | wc -l | tr -d ' ')" = 1 ] || { echo "make-dmg: not one qt-<version>.sha256 in $here" >&2; exit 1; }
qtver=$(basename "$manifest" .sha256); qtver=${qtver#qt-}
prefix=${QT_PREFIX:-$HOME/qtmac-tools/qt-$qtver}
"$here/fetch-qt.sh" "$manifest" "$prefix"
export PATH="$prefix/CMake.app/Contents/bin:$prefix:$HOME/.cargo/bin:$PATH"
export LANG=en_US.UTF-8   # Qt's tools want a UTF-8 locale; over ssh it is "C"
[ "$("$prefix/bin/qtpaths" --qt-version)" = "$qtver" ] || { echo "make-dmg: $prefix is not Qt $qtver" >&2; exit 1; }
# Qt's own minimum macOS, for the app, the core and LSMinimumSystemVersion
min=$(sed -n 's/.*set(QT_SUPPORTED_MIN_MACOS_VERSION "\([0-9.]*\)").*/\1/p' "$prefix/lib/cmake/Qt6/Qt6ConfigExtras.cmake" | head -n 1)
[ -n "$min" ] || { echo "make-dmg: Qt names no minimum macOS" >&2; exit 1; }
case $min in *.*) ;; *) min=$min.0 ;; esac
export MACOSX_DEPLOYMENT_TARGET=$min
build=${BUILD_DIR:-$out/build}
target=${CARGO_TARGET_DIR:-$out/target}
feed=${DSPI_UPDATE_FEED_URL-https://api.github.com/repos/Bofr5555/dspi-unified-console/releases?per_page=30}
version=$(sed -n 's/^project(uconsole-for-dspi VERSION \([0-9.]*\).*/\1/p' "$root/qt/CMakeLists.txt")
[ -n "$version" ] || { echo "make-dmg: no version in qt/CMakeLists.txt" >&2; exit 1; }
name=$(sed -n 's/.*displayName = "\(.*\)";/\1/p' "$root/qt/src/AppIdentity.h")
[ -n "$name" ] || { echo "make-dmg: no displayName in qt/src/AppIdentity.h" >&2; exit 1; }
dmg="$out/uConsole-for-DSPi-$version-macos.dmg"

# The core for each architecture (ci/check.sh's CAPI_FEATURES), then one
# universal static library
libs=()
for t in x86_64-apple-darwin aarch64-apple-darwin; do
    (cd "$repo" && CARGO_TARGET_DIR="$target" cargo build --release --locked -p dspi-capi \
        --features link,midir,mdns,hub --target "$t")
    libs+=("$target/$t/release/libdspi_capi.a")
done
mkdir -p "$target/universal-apple-darwin/release"
lipo -create "${libs[@]}" -output "$target/universal-apple-darwin/release/libdspi_capi.a"

rm -rf "$build"
cmake -S "$root/qt" -B "$build" -G Ninja -DCMAKE_BUILD_TYPE=Release \
      -DCMAKE_PREFIX_PATH="$prefix" -DCMAKE_OSX_ARCHITECTURES="x86_64;arm64" \
      -DCMAKE_OSX_DEPLOYMENT_TARGET="$min" \
      -DDSPI_CARGO_TARGET_DIR="$target" -DDSPI_RUST_TARGET=universal-apple-darwin \
      -DDSPI_UPDATE_FEED_URL="$feed"
cmake --build "$build" -j"${JOBS:-4}" --target uconsole-for-dspi

stage=$out/dmg-root
app="$stage/$name.app"
exe="$app/Contents/MacOS/$name"
rm -rf "$stage" "$dmg"; mkdir -p "$stage"
cp -R "$build/$name.app" "$app"
"$prefix/bin/macdeployqt" "$app" -qmldir="$root/qt/qml" -always-overwrite
# Not in the package (see above): Qt Multimedia's media plugins and the FFmpeg
# libraries they bring, and the SQL drivers (QtQuick.LocalStorage's; macdeployqt
# reports the client libraries they want and a Mac lacks)
rm -rf "$app/Contents/PlugIns/multimedia" "$app/Contents/PlugIns/sqldrivers"
rm -f "$app"/Contents/Frameworks/libav*.dylib "$app"/Contents/Frameworks/libsw*.dylib
if find "$app" -iname 'libav*' -o -iname 'libsw*' -o -iname '*ffmpeg*' | grep -q .; then
    echo "make-dmg: FFmpeg files in the bundle:" >&2; find "$app" -iname 'libav*' -o -iname 'libsw*' -o -iname '*ffmpeg*' >&2; exit 1
fi
[ -d "$app/Contents/Frameworks/QtMultimedia.framework" ] || { echo "make-dmg: QtMultimedia.framework is missing (the microphone)" >&2; exit 1; }

# Qt dynamic and from the bundle (the LGPL lets users swap it), and nothing
# else linked but the system's
otool -L "$exe" | grep -q '@rpath/QtCore.framework/Versions/A/QtCore' \
    || { echo "make-dmg: the app does not link QtCore dynamically" >&2; exit 1; }
[ -f "$app/Contents/Frameworks/QtCore.framework/Versions/A/QtCore" ] \
    || { echo "make-dmg: QtCore.framework is not in the bundle" >&2; exit 1; }
foreign=$(find "$app/Contents/MacOS" "$app/Contents/Frameworks" "$app/Contents/PlugIns" -type f -perm -u+x \
          -exec otool -L {} + 2>/dev/null | grep -v ':$' | awk '{print $1}' \
          | grep -v -e '^@rpath/' -e '^@executable_path/' -e '^@loader_path/' -e '^/System/Library/' -e '^/usr/lib/' | sort -u || true)
[ -z "$foreign" ] || { echo "make-dmg: libraries from outside the bundle and the system:" >&2; echo "$foreign" >&2; exit 1; }
if otool -l "$exe" | grep -A2 LC_RPATH | grep -q -e "$prefix" -e "$HOME"; then
    echo "make-dmg: the app still looks for libraries under $prefix or $HOME" >&2; exit 1
fi

# The licence texts, the notice and the list of bundled libraries (before
# signing: the signature covers them)
QTDIR=$prefix "$root/packaging/licences/stage.sh" macos "$app" "$app/Contents/Resources" "$version"

# The hardened runtime (needed for notarisation) only with a real identity:
# ad hoc, its library validation would refuse the bundled Qt frameworks
if [ -n "${SIGN_ID:-}" ] && [ "$SIGN_ID" != - ]; then
    codesign --force --deep --options runtime --timestamp --sign "$SIGN_ID" "$app"
else
    codesign --force --deep --sign - "$app"
fi
codesign --verify --deep --strict "$app"

lipo -info "$exe"
echo "make-dmg: minimum macOS $(otool -l "$exe" | awk '/LC_BUILD_VERSION/{f=1} f && /minos/{print $2; exit}')"

# The .dmg: the app and a link to /Applications to drag it onto
ln -s /Applications "$stage/Applications"
hdiutil create -quiet -volname "$name" -srcfolder "$stage" -fs HFS+ -format UDZO "$dmg"
rm -rf "$stage"
echo "make-dmg: $dmg"
