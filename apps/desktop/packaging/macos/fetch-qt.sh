#!/bin/bash
# Qt 6 for macOS, with Qt's CMake and Ninja, straight from Qt's own package
# repository on download.qt.io: no Qt account, no installer, nothing written
# outside the target directory. For build-macos.sh in the CI VM (ci/macvm.sh qt-dmg).
#
#   apps/desktop/packaging/macos/fetch-qt.sh [dir, default ~/work/qt]
#
# Leaves Qt in <dir>/6.11.2/macos (universal: x86_64 and arm64), CMake in
# <dir>/Tools/CMake and Ninja in <dir>/Tools/Ninja, and prints the three
# directories as shell assignments (QTDIR, CMAKE_DIR, NINJA_DIR).
#
# Every archive is checked twice: against the SHA-1 Qt publishes next to it
# (fetched from download.qt.io itself, never from a mirror; the archives may
# come from a mirror) and against the SHA-256 pinned below. A second run
# finds the stamps and does nothing.
set -euo pipefail
dir=${1:-$HOME/work/qt}
repo=https://download.qt.io/online/qtsdkrepository/mac_x64/desktop
qtver=6.11.2

# destination (relative to dir) | path in the repository | pinned SHA-256
qt=qt6_6112/qt6_6112/qt.qt6.6112.clang_64/6.11.2-0-202608131016
suffix=MacOS-MacOS_15-Clang-MacOS-MacOS_15-X86_64-ARM64.7z
archives=(
  "$qtver/macos|$qt""qtbase-$suffix|9592f84f7e26d532c5c56824d1da7c9214a766cb0a17beb5af71022bcfbcd271"
  "$qtver/macos|$qt""qtdeclarative-$suffix|ceb8e3f3830531a52de5007ef3f50ffc5525021feb132ee322f59583eb2903cf"
  "$qtver/macos|$qt""qtsvg-$suffix|c52f6cec6ce4b52ca017e0046345a85d33b2b73e0f6a93474bb7a60f8e829317"
  "Tools/CMake|tools_cmake/qt.tools.cmake/3.30.5-202502251255macos64_x86_cmake_tools.7z|6bd1998d51a72b36468de58c7ad916bcd03c9778ab05a9a9382efbf9640649e5"
  "Tools/Ninja|tools_ninja/qt.tools.ninja/1.12.1-202502251256macos64_ninja.7z|a320b4b41dcef46730c4e617add8a945257f4d4baaccb8753da53f8863c8dd85"
)

mkdir -p "$dir/downloads"
for a in "${archives[@]}"; do
    IFS='|' read -r dest path pin <<<"$a"
    file=${path##*/}
    stamp="$dir/downloads/$file.done"
    [ -f "$stamp" ] && continue
    # The published SHA-1, from download.qt.io only: no redirect allowed
    sha1=$(curl -fsS --max-redirs 0 "$repo/$path.sha1" | cut -c1-40)
    [[ "$sha1" =~ ^[0-9a-f]{40}$ ]] || { echo "fetch-qt: no SHA-1 for $file" >&2; exit 1; }
    curl -fsSL --retry 3 -o "$dir/downloads/$file" "$repo/$path"
    got1=$(shasum -a 1 "$dir/downloads/$file" | cut -d' ' -f1)
    got256=$(shasum -a 256 "$dir/downloads/$file" | cut -d' ' -f1)
    if [ "$got1" != "$sha1" ]; then
        echo "fetch-qt: $file: SHA-1 $got1, Qt publishes $sha1" >&2; exit 1
    fi
    if [ "$got256" != "$pin" ]; then
        echo "fetch-qt: $file: SHA-256 $got256, pinned ${pin:-nothing}" >&2; exit 1
    fi
    mkdir -p "$dir/$dest"
    tar -xf "$dir/downloads/$file" -C "$dir/$dest"   # bsdtar reads 7z
    rm -f "$dir/downloads/$file"
    echo "$got256" >"$stamp"
    echo "fetch-qt: $file verified and unpacked" >&2
done

# The prebuilt Qt has its build machine's prefix compiled in; qt.conf points
# qtpaths (which macdeployqt asks) at this copy, as Qt's installer does
qtdir=$dir/$qtver/macos
printf '[Paths]\nPrefix=..\n' >"$qtdir/bin/qt.conf"
echo "QTDIR=$qtdir"
echo "CMAKE_DIR=$dir/Tools/CMake/CMake.app/Contents/bin"
echo "NINJA_DIR=$dir/Tools/Ninja"
