#!/bin/bash
# Windows x86_64 release from Linux: the core cross-compiled with cargo
# (x86_64-pc-windows-gnu), the app with MinGW Qt 6, then a folder that runs
# as is, a .zip of it and a per-user installer. Run inside the build image
# (Fedora with mingw64-qt6-*, mingw64-gcc-c++, mingw32-nsis, rustup's
# x86_64-pc-windows-gnu target): packaging/windows/Containerfile, or the CI
# image (ci/check.sh --step qt-windows).
#
#   packaging/windows/build-windows.sh [output dir, default dist/]
#
# -- WP QP20 -- The files it writes keep the names the app's update checker
# looks for (packaging/README.md, "Release files"):
#   uConsole-for-DSPi-<version>-windows-x86_64.zip, ...-windows-x86_64-setup.exe
# This is one of the project's own builds, so it carries the update feed
# (DSPI_UPDATE_FEED_URL; set it empty for a build that never checks).
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)          # apps/desktop
repo=$(cd "$root/../.." && pwd)                      # the dspi-unified-console root
out=${1:-$repo/dist}
target=${CARGO_TARGET_DIR:-$repo/target}
build=${BUILD_DIR:-$root/qt/build-win64}   # (gitignored)
triple=x86_64-pc-windows-gnu
feed=${DSPI_UPDATE_FEED_URL-https://api.github.com/repos/Bofr5555/dspi-unified-console/releases?per_page=30}

# -- WP QP20 -- The version is the CMake project's (AppIdentity's version)
version=$(sed -n 's/^project(uconsole-for-dspi VERSION \([0-9.]*\).*/\1/p' "$root/qt/CMakeLists.txt")
[ -n "$version" ] || { echo "build-windows: no version in qt/CMakeLists.txt's project()" >&2; exit 1; }
name="uConsole-for-DSPi-$version-windows-x86_64"

(cd "$repo" && cargo build --release --locked -p dspi-capi --features link,midir,mdns,hub --target $triple)
cmake -S "$root/qt" -B "$build" -G Ninja -DCMAKE_BUILD_TYPE=Release \
      -DCMAKE_TOOLCHAIN_FILE=/usr/share/mingw/toolchain-mingw64.cmake \
      -DDSPI_CARGO_TARGET_DIR="$target" -DDSPI_RUST_TARGET=$triple \
      -DDSPI_UPDATE_FEED_URL="$feed"
# -- WP QD21c -- The QML is compiled ahead of time by the image's host
# qmlcachegen; the app loads those units only on the Qt version that made them
# and otherwise compiles every QML file from source again, silently. So the
# host qmlcachegen must be the MinGW Qt's version.
cachegen=$(grep -o '[^ ]*/qmlcachegen' "$build/build.ninja" | head -n 1 || true)
if [ -n "$cachegen" ]; then
    core_dir=$(sed -n 's/^Qt6Core_DIR:PATH=//p' "$build/CMakeCache.txt")
    qt_ver=$(sed -n 's/^set(PACKAGE_VERSION "\([0-9.]*\)")$/\1/p' "$core_dir"/Qt6CoreConfigVersion*.cmake 2>/dev/null | head -n 1)
    gen_ver=$("$cachegen" --version 2>/dev/null | grep -o '[0-9][0-9.]*' | head -n 1 || true)
    if [ -z "$qt_ver" ] || [ -z "$gen_ver" ]; then
        echo "build-windows: cannot tell the Qt versions (MinGW Qt '$qt_ver', $cachegen '$gen_ver')" >&2
        exit 1
    elif [ "$qt_ver" != "$gen_ver" ]; then
        echo "build-windows: $cachegen is Qt $gen_ver but the MinGW Qt is $qt_ver: the app would ignore its compiled QML." >&2
        echo "build-windows: install the matching host Qt (qt6-qtdeclarative-devel) in the image" >&2
        exit 1
    fi
    echo "build-windows: qmlcachegen and the MinGW Qt are both Qt $qt_ver"
fi
cmake --build "$build" --target uconsole-for-dspi
# -- WP QD21a -- Qt stays dynamic (LGPL-3: users can swap the Qt DLLs)
imports=$(x86_64-w64-mingw32-objdump -p "$build/uconsole-for-dspi.exe")
grep -q 'DLL Name: Qt6Core.dll' <<<"$imports" \
    || { echo "build-windows: uconsole-for-dspi.exe does not load Qt6Core.dll (Qt linked statically?)" >&2; exit 1; }
# -- WP QP20 -- and carries its icon and version information (uconsole-for-dspi.rc.in)
x86_64-w64-mingw32-objdump -h "$build/uconsole-for-dspi.exe" | grep -q '\.rsrc' \
    || { echo "build-windows: uconsole-for-dspi.exe has no resources (icon, version)" >&2; exit 1; }

mkdir -p "$out"
"$root/packaging/windows/deploy-mingw.sh" "$build" "$out/$name"
# -- WP QD42 -- The licence texts (Qt's, GCC's runtime and the other DLLs'), the
# notice and the list of bundled libraries, in the zip and the installer
"$root/packaging/licences/stage.sh" windows "$out/$name" "$out/$name" "$version"
(cd "$out" && rm -f "$name.zip" && python3 -m zipfile -c "$name.zip" "$name")
makensis -V2 -DVERSION="$version" -DBUNDLE="$out/$name" -DOUTFILE="$out/$name-setup.exe" \
         "$root/packaging/windows/uconsole-for-dspi.nsi"
ls -la "$out/$name.zip" "$out/$name-setup.exe"
