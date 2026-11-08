#!/bin/bash
# Windows x86_64 release from Linux: the core cross-compiled with cargo
# (x86_64-pc-windows-gnu), the app with MinGW Qt 6, then a folder that runs
# as is, a .zip of it and a per-user installer. Run inside the build image
# (Fedora with mingw64-qt6-*, mingw64-gcc-c++, mingw32-nsis, rustup's
# x86_64-pc-windows-gnu target): packaging/windows/Containerfile.
#
#   packaging/windows/build-windows.sh [output dir, default dist/]
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)          # apps/desktop
repo=$(cd "$root/../.." && pwd)                      # the dspi-unified-console root
out=${1:-$repo/dist}
target=${CARGO_TARGET_DIR:-$repo/target}
build=${BUILD_DIR:-$root/qt/build-win64}   # (gitignored)
triple=x86_64-pc-windows-gnu

version=$(sed -n 's/.*setApplicationVersion("\([^"]*\)").*/\1/p' "$root/qt/src/main.cpp")
name="DSPi-Console-$version-windows-x86_64"

(cd "$repo" && cargo build --release -p dspi-capi --features link,midir,mdns,hub --target $triple)
cmake -S "$root/qt" -B "$build" -G Ninja -DCMAKE_BUILD_TYPE=Release \
      -DCMAKE_TOOLCHAIN_FILE=/usr/share/mingw/toolchain-mingw64.cmake \
      -DDSPI_QT_MAJOR=6 -DDSPI_CARGO_TARGET_DIR="$target" -DDSPI_RUST_TARGET=$triple
# -- WP QD21c -- The precompiled QML (DSPI_QML_PRECOMPILE) comes from the image's
# host qmlcachegen; the app loads those units only on the Qt version that made
# them and otherwise compiles every QML file from source again, silently. So the
# host qmlcachegen must be the MinGW Qt's version.
cachegen=$(grep -o '[^ ]*/qmlcachegen' "$build/build.ninja" | head -n 1 || true)
if [ -n "$cachegen" ]; then
    core_dir=$(sed -n 's/^Qt6Core_DIR:PATH=//p' "$build/CMakeCache.txt")
    qt_ver=$(sed -n 's/^set(PACKAGE_VERSION "\([0-9.]*\)")$/\1/p' "$core_dir"/Qt6CoreConfigVersion*.cmake 2>/dev/null | head -n 1)
    gen_ver=$("$cachegen" --version 2>/dev/null | grep -o '[0-9][0-9.]*' | head -n 1 || true)
    if [ -z "$qt_ver" ] || [ -z "$gen_ver" ]; then
        echo "build-windows: cannot tell the Qt versions (MinGW Qt '$qt_ver', $cachegen '$gen_ver'); refusing to build precompiled QML that may be ignored" >&2
        exit 1
    elif [ "$qt_ver" != "$gen_ver" ]; then
        echo "build-windows: $cachegen is Qt $gen_ver but the MinGW Qt is $qt_ver: the app would ignore its precompiled QML." >&2
        echo "build-windows: install the matching host Qt in the image, or configure $build once with -DDSPI_QML_PRECOMPILE=OFF" >&2
        exit 1
    fi
    echo "build-windows: qmlcachegen and the MinGW Qt are both Qt $qt_ver"
fi
cmake --build "$build"
# -- WP QD21a -- Qt stays dynamic (LGPL-3: users can swap the Qt DLLs)
imports=$(x86_64-w64-mingw32-objdump -p "$build/DSPiConsole.exe")
grep -q 'DLL Name: Qt6Core.dll' <<<"$imports" \
    || { echo "build-windows: DSPiConsole.exe does not load Qt6Core.dll (Qt linked statically?)" >&2; exit 1; }

mkdir -p "$out"
"$root/packaging/windows/deploy-mingw.sh" "$build" "$out/$name" 6
# -- WP QD42 -- The licence texts (Qt's, GCC's runtime and the other DLLs'), the
# notice and the list of bundled libraries, in the zip and the installer
"$root/packaging/licences/stage.sh" windows "$out/$name" "$out/$name" "$version"
(cd "$out" && rm -f "$name.zip" && python3 -m zipfile -c "$name.zip" "$name")
makensis -V2 -DVERSION="$version" -DBUNDLE="$out/$name" -DOUTFILE="$out/$name-setup.exe" \
         "$root/packaging/windows/dspi-console.nsi"
ls -la "$out/$name.zip" "$out/$name-setup.exe"
