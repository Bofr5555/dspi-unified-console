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
cmake --build "$build"

mkdir -p "$out"
"$root/packaging/windows/deploy-mingw.sh" "$build" "$out/$name" 6
(cd "$out" && rm -f "$name.zip" && python3 -m zipfile -c "$name.zip" "$name")
makensis -V2 -DVERSION="$version" -DBUNDLE="$out/$name" -DOUTFILE="$out/$name-setup.exe" \
         "$root/packaging/windows/dspi-console.nsi"
ls -la "$out/$name.zip" "$out/$name-setup.exe"
