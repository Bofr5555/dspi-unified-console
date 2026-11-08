#!/usr/bin/env bash
# Installs the official Qt binaries listed in a manifest (qt-<version>.sha256)
# from download.qt.io into a prefix:
#   fetch-qt.sh <manifest> <prefix>     e.g. fetch-qt.sh qt-6.8.3.sha256 /opt/qt
# Each archive must match the SHA-256 pinned in the manifest and the one
# download.qt.io publishes next to it (<file>.sha256); any mismatch fails.
# Qt stays a set of shared libraries (LGPL-3); nothing is linked statically.
set -euo pipefail
manifest=$1; prefix=$2
base=https://download.qt.io/online/qtsdkrepository/linux_x64/desktop
dl=$(mktemp -d)
mkdir -p "$prefix"
grep -v '^#' "$manifest" | while read -r sum path; do
    [ -n "$sum" ] || continue
    f="$dl/${path##*/}"
    curl -sSfL --retry 4 --retry-all-errors -o "$f" "$base/$path"
    published=$(curl -sSfL --retry 4 --retry-all-errors "$base/$path.sha256" | awk '{print $1}')
    [ "$published" = "$sum" ] || { echo "fetch-qt: download.qt.io states $published for $path, pinned $sum" >&2; exit 1; }
    echo "$sum  $f" | sha256sum -c --quiet - || { echo "fetch-qt: $path does not match its SHA-256" >&2; exit 1; }
    echo "fetch-qt: $path: SHA-256 $sum (pinned, as published)"
    case $f in *.7z) 7z x -y -bd -o"$prefix" "$f" >/dev/null ;; esac
    rm -f "$f"
done
rm -rf "$dl"
# The archives unpack as the Qt installer lays them out: Qt 6.5-6.7 under
# <version>/gcc_64, 6.8 at the top; ICU at the top in both. Flatten that to
# <prefix>/{bin,lib,...} with ICU in lib/, as the installer does.
for d in "$prefix"/6.*/gcc_64; do
    [ -d "$d" ] || continue
    cp -a "$d"/. "$prefix"/ && rm -rf "${d%/gcc_64}"
done
find "$prefix" -maxdepth 1 -name 'libicu*.so*' -exec mv -t "$prefix/lib" {} +
"$prefix/bin/qtpaths" --qt-version >/dev/null || { echo "fetch-qt: $prefix/bin/qtpaths does not run" >&2; exit 1; }
echo "fetch-qt: Qt $("$prefix/bin/qtpaths" --qt-version) in $prefix"
