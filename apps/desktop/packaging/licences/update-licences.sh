#!/bin/bash
# -- WP QD42 -- Refreshes the licence texts in this folder, which the package
# builds copy into the AppImage, the Windows zip and installer, the .dmg, the
# .deb and the .rpm (stage.sh). Run after release-sources.sh, whenever a pinned
# Qt, KWindowSystem or ICU version changes; commit the result. Every text is
# copied verbatim from its official source:
#   GPL-3.0.txt, LGPL-3.0.txt   www.gnu.org/licenses (GPL-3.0.txt must equal the
#                               repository's LICENSE)
#   qt-<version>/               Qt's licence files and third-party notices, from the
#                               source archives release-sources.sh checked
#                               (qt-third-party.py), one folder per Qt the
#                               packages bundle (qt-modules.sh)
#   kwindowsystem-<version>/    KWindowSystem's LICENSES/, from its checked archive
#   icu-<version>/LICENSE       ICU's licence, from its checked source archive
#
#   apps/desktop/packaging/licences/update-licences.sh [release-sources dir]
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../../.." && pwd)
src=${1:-${RELEASE_SOURCES:-$HOME/.cache/dspi-build/work/release-sources}}
fail() { echo "update-licences: $*" >&2; exit 1; }

for l in gpl-3.0 lgpl-3.0; do
    f=$here/$(tr a-z A-Z <<<"${l%-*}")-3.0.txt
    curl -sSf --max-redirs 0 -o "$f" "https://www.gnu.org/licenses/$l.txt"
done
cmp -s "$here/GPL-3.0.txt" "$repo/LICENSE" || fail "www.gnu.org's GPL-3.0 differs from the repository's LICENSE"

for kind in appimage macos; do
    line=$("$here/qt-modules.sh" "$kind"); v=${line%% *}
    archives=()
    for m in ${line#* }; do
        a=$src/qt/$m-everywhere-src-$v.tar.xz
        [ -f "$a" ] || fail "no $a: run release-sources.sh first"
        archives+=("$a")
    done
    rm -rf "$here/qt-$v"
    python3 -I "$here/qt-third-party.py" "$here/qt-$v" "${archives[@]}"
done

for a in "$src"/kde/kwindowsystem-*.tar.xz; do
    [ -f "$a" ] || fail "no KWindowSystem archive in $src/kde: run release-sources.sh first"
    v=$(basename "$a" .tar.xz); v=${v#kwindowsystem-}
    rm -rf "$here/kwindowsystem-$v"; mkdir -p "$here/kwindowsystem-$v"
    tar -xJf "$a" -C "$here/kwindowsystem-$v" --strip-components=1 --wildcards "kwindowsystem-$v/LICENSES/*"
done

for a in "$src"/icu/icu4c-*-src.tgz; do
    [ -f "$a" ] || continue
    v=$(basename "$a" -src.tgz); v=${v#icu4c-}; v=${v//_/.}
    rm -rf "$here/icu-$v"; mkdir -p "$here/icu-$v"
    tar -xzOf "$a" icu/LICENSE >"$here/icu-$v/LICENSE"
done
echo "update-licences: done; review and commit $here"
