#!/bin/bash
# -- WP QTMAC -- Installs the official Qt binaries for macOS and Qt's CMake and
# Ninja, as listed in a manifest (qt-<version>.sha256), from download.qt.io into
# a prefix: linux/fetch-qt.sh's rules, on the Mac (bash 3.2, shasum, bsdtar):
#
#   fetch-qt.sh <manifest> <prefix>     e.g. fetch-qt.sh qt-6.8.3.sha256 ~/qtmac-tools/qt-6.8.3
#
# Each archive must match the SHA-256 pinned in the manifest and the one
# download.qt.io publishes next to it (<file>.sha256, read from download.qt.io
# itself, no redirect); any mismatch fails. Qt stays a set of frameworks
# (LGPL-3); nothing is linked statically. A second run with the same manifest
# finds its stamp and does nothing. Leaves Qt at <prefix> (bin, lib, plugins,
# qml), CMake at <prefix>/CMake.app/Contents/bin and Ninja at <prefix>/ninja.
set -euo pipefail
manifest=$1; prefix=$2
base=https://download.qt.io/online/qtsdkrepository/mac_x64/desktop
want=$(shasum -a 256 "$manifest" | cut -d' ' -f1)
if [ "$(cat "$prefix/.fetched" 2>/dev/null)" = "$want" ]; then
    echo "fetch-qt: $prefix is up to date ($manifest)" >&2; exit 0
fi
rm -rf "$prefix"; mkdir -p "$prefix/.downloads"
grep -v '^#' "$manifest" | while read -r sum path; do
    [ -n "$sum" ] || continue
    f="$prefix/.downloads/${path##*/}"
    curl -sSfL --retry 4 -o "$f" "$base/$path"
    published=$(curl -sSf --max-redirs 0 --retry 4 "$base/$path.sha256" | awk '{print $1}')
    [ "$published" = "$sum" ] || { echo "fetch-qt: download.qt.io states $published for $path, pinned $sum" >&2; exit 1; }
    echo "$sum  $f" | shasum -a 256 -c --quiet - || { echo "fetch-qt: $path does not match its SHA-256" >&2; exit 1; }
    echo "fetch-qt: $path: SHA-256 $sum (pinned, as published)" >&2
    tar -xf "$f" -C "$prefix"   # bsdtar reads 7z
    rm -f "$f"
done
rmdir "$prefix/.downloads"
# The prebuilt Qt has its build machine's prefix compiled in; qt.conf points
# qtpaths (which macdeployqt asks) at this copy, as Qt's installer does
printf '[Paths]\nPrefix=..\n' >"$prefix/bin/qt.conf"
"$prefix/bin/qtpaths" --qt-version >/dev/null || { echo "fetch-qt: $prefix/bin/qtpaths does not run" >&2; exit 1; }
echo "$want" >"$prefix/.fetched"
echo "fetch-qt: Qt $("$prefix/bin/qtpaths" --qt-version) in $prefix" >&2
