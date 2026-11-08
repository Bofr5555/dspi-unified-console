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
#   appimage-runtime-<commit>/  the AppImage runtime's LICENSE, libfuse's LICENSE and
#                               LGPL2.txt (the LGPL-2.1), squashfuse's LICENSE, from
#                               their checked archives, and SOURCES.txt
#                               (the runtime's commit and the archives it builds on)
#   fedora/<source>-<version>/  the licence file of a Fedora MinGW package that
#                               ships none (FEDORA_NO_LICENCE below), from the
#                               upstream archive in its checked source RPM
#
#   apps/desktop/packaging/licences/update-licences.sh [release-sources dir]
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../../.." && pwd)
src=${1:-${RELEASE_SOURCES:-$HOME/.cache/dspi-build/work/release-sources}}
fail() { echo "update-licences: $*" >&2; exit 1; }

# (KEEP_GNU_TEXTS=1 keeps the committed copies, e.g. while www.gnu.org is unreachable)
for l in gpl-3.0 lgpl-3.0; do
    [ "${KEEP_GNU_TEXTS:-}" = 1 ] && break
    f=$here/$(tr a-z A-Z <<<"${l%-*}")-${l##*-}.txt
    curl -sSf --max-redirs 0 --retry 2 --max-time 60 -o "$f.part" "https://www.gnu.org/licenses/$l.txt" \
        || { rm -f "$f.part"; fail "cannot read www.gnu.org's $l.txt (KEEP_GNU_TEXTS=1 keeps the committed copy)"; }
    grep -q 'GENERAL PUBLIC LICENSE' "$f.part" || fail "www.gnu.org's $l.txt is not a licence text"
    mv "$f.part" "$f"
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
for a in "$src"/runtime/type2-runtime-*.tar.gz; do
    [ -f "$a" ] || continue
    full=$(basename "$a" .tar.gz); full=${full#type2-runtime-}
    d=$here/appimage-runtime-${full:0:7}
    rm -rf "$d"; mkdir -p "$d"
    tar -xzOf "$a" "type2-runtime-$full/LICENSE" >"$d/LICENSE"
    deps=$(tar -xzOf "$a" "type2-runtime-$full/scripts/common/install-dependencies.sh")
    sq=$(ls "$src"/runtime/squashfuse-*.tar.gz | head -n 1)
    tar -xzOf "$sq" --wildcards '*/LICENSE' >"$d/squashfuse-LICENSE"
    fz=$(ls "$src"/runtime/fuse-*.tar.xz | head -n 1)
    tar -xJOf "$fz" --wildcards '*/LICENSE' >"$d/libfuse-LICENSE"
    tar -xJOf "$fz" --wildcards '*/LGPL2.txt' >"$d/libfuse-LGPL2.txt"
    grep -q 'GNU LESSER GENERAL PUBLIC LICENSE' "$d/libfuse-LGPL2.txt" || fail "$fz has no LGPL in LGPL2.txt"
    {
        echo "type2-runtime-$full.tar.gz (https://github.com/AppImage/type2-runtime, commit $full)"
        sed -n 's/^wget "\{0,1\}\([^" ]*\)"\{0,1\}$/\1/p' <<<"$deps"
    } >"$d/SOURCES.txt"
done

# Fedora MinGW packages whose RPM ships no licence file: <source package> <file in its upstream archive>
FEDORA_NO_LICENCE="mingw-bzip2:LICENSE mingw-openssl:LICENSE.txt mingw-zlib:LICENSE mingw-win-iconv:readme.txt"
for e in $FEDORA_NO_LICENCE; do
    name=${e%%:*}; file=${e#*:}
    for srpm in "$src"/fedora/"$name"-[0-9]*.src.rpm; do
        [ -f "$srpm" ] || fail "no $name source RPM in $src/fedora: run release-sources.sh with the Windows dist first"
        nvr=$(basename "$srpm" .src.rpm); ver=${nvr#"$name"-}; ver=${ver%-*}
        rm -rf "$here/fedora/$name-$ver"; mkdir -p "$here/fedora/$name-$ver"
        podman run --rm --network=none -v "$src/fedora":/s:ro,z -v "$here/fedora/$name-$ver":/o:z registry.fedoraproject.org/fedora:44 bash -c '
            set -e; cd "$(mktemp -d)"; rpm2archive - <"/s/$1" | tar -xzf -
            a=$(ls ./*-"$2".tar.* | head -n 1)
            tar -xOf "$a" --wildcards "*-$2/$3" >"/o/$3"
            [ -s "/o/$3" ]' _ "$(basename "$srpm")" "$ver" "$file" || fail "no $file in $srpm"
    done
done
echo "update-licences: done; review and commit $here"
