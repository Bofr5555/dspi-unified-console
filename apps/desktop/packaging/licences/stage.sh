#!/bin/bash
# -- WP QD42 -- Puts the licence texts and the notice into a package that
# bundles Qt, at <dest>: NOTICE.txt and LICENSES/ with
#   GPL-3.0.txt, LGPL-3.0.txt   DSPi Console's licence, and Qt's (from www.gnu.org)
#   TERMS.txt                   (WP LIC1) DSPi Console's additional terms under
#                               GPL-3.0 section 7, and its copyright (TERMS.txt here)
#   rust-crates.txt             (WP LIC1) the Rust crates in its core and their
#                               licence texts (gen_rust_notices.py)
#   Qt/                         Qt's LICENSES/ and THIRD-PARTY.txt (qt-<version>/)
#   KWindowSystem/, ICU/        (AppImage) theirs
#   AppImage-runtime/           (AppImage) the runtime's, and those of what it links
#                               (libfuse, squashfuse; WP LIC1: musl, zlib, zstd and
#                               mimalloc from Alpine, ALPINE.txt)
#   BUNDLED.txt                 every library in the package that is not DSPi
#                               Console's own: package, version, where it comes
#                               from, its source, its licence, its files
#                               (tab-separated); release-sources.sh fetches the
#                               sources it names
#   bundled/<package>/          (AppImage, Windows) the licence files the
#                               distribution ships with each such package
#
#   stage.sh appimage <AppDir> <dest> <app version>   in Containerfile.ubuntu's image
#   stage.sh windows <bundle> <dest> <app version>    in the MinGW build image (Fedora)
#   stage.sh macos <app bundle> <dest> <app version>  on the Mac (bash 3.2)
#   stage.sh swift <app bundle> <dest> <app version>  (WP REL013M) the Swift macOS
#                               app, on the Mac: no Qt, no LGPL-3.0.txt; its core's
#                               crates (apps/apple/licences/rust-crates.txt) and the
#                               Swift packages linked into it, from $PACKAGE_RESOLVED
#                               with their licence files from $SWIFTPM_CHECKOUTS
# Fails on a file in the package it cannot account for, and when this folder
# has no licence texts for the Qt (or KWindowSystem, ICU) the package carries
# (update-licences.sh adds them).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
kind=${1:?usage: stage.sh appimage|windows|macos|swift <package dir> <dest> <app version>}
pkg=${2:?}; dest=${3:?}; version=${4:?}
fail() { echo "stage-licences: $*" >&2; exit 1; }
L=$dest/LICENSES
rm -rf "$L"; mkdir -p "$L"
cp "$here/GPL-3.0.txt" "$here/LGPL-3.0.txt" "$L/"
cp "$here/TERMS.txt" "$here/rust-crates.txt" "$L/"   # -- WP LIC1 --
# -- WP CLEAN1 -- the bundled firmware images' third-party notices, and AutoEq's
# licence (the AutoEQ database ships in every package)
cp "$here/../../../../assets/licences/firmware-THIRD-PARTY-NOTICES.txt" \
   "$here/../../../../assets/licences/AutoEq-MIT.txt" "$L/"
# -- WP CLEAN1 -- the release's source archive (ci/release-source.sh), which the
# notice names: published beside the downloads, not a link into the repository
source_asset=${DSPI_SOURCE_ASSET:-dspi-uconsole-$version-source.tar.gz}
tab=$(printf '\t')
bundled=$L/BUNDLED.txt
printf '# Libraries in this package that are not part of DSPi uConsole (tab-separated):\n# package\tversion\tfrom\tsource\tlicence\tfiles\n' >"$bundled"
row() { printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$@" >>"$bundled"; }
copy_dir() {  # <licences/ subfolder> <dest name> <what>
    [ -d "$here/$1" ] || fail "no licence texts for $3 in $here/$1 (run update-licences.sh)"
    mkdir -p "$L/$2"; cp -R "$here/$1"/. "$L/$2/"
}
qt_sources() {  # <qt-modules.sh kind>: the source archive names
    local line m s=
    line=$("$here/qt-modules.sh" "$1") || return 1
    for m in ${line#* }; do s="$s $m-everywhere-src-${line%% *}.tar.xz"; done
    echo "${s# }"
}
join() { tr '\n' ' ' | sed 's/ $//'; }

case $kind in
appimage)
    Q=${QTDIR:-/opt/qt}; K=${KF_DIR:-/opt/kf6}
    qt=$("$Q/bin/qtpaths" --qt-version)
    [ "$("$here/qt-modules.sh" appimage | cut -d' ' -f1)" = "$qt" ] || fail "the image's Qt is $qt, the pins say otherwise"
    copy_dir "qt-$qt" Qt "Qt $qt"
    qtfiles=; icufiles=; kfiles=; debs=
    for f in "$pkg"/usr/lib/*.so*; do
        b=${f##*/}
        if [ -e "$Q/lib/$b" ]; then
            case $b in libicu*) icufiles="$icufiles usr/lib/$b" ;; *) qtfiles="$qtfiles usr/lib/$b" ;; esac
        elif [ -e "$K/lib/$b" ]; then
            kfiles="$kfiles usr/lib/$b"
        else
            p=$(dpkg -S "*/x86_64-linux-gnu/$b" 2>/dev/null | head -n 1 | cut -d: -f1) || true
            [ -n "$p" ] || fail "usr/lib/$b comes from neither Qt, KWindowSystem nor an Ubuntu package"
            debs="$debs$p $b"$'\n'
        fi
    done
    # Plugins and QML modules: Qt's, and KWindowSystem's plugins
    while read -r f; do
        rel=${f#"$pkg"/usr/}
        case $rel in
            plugins/kf6/*) [ -e "$K/lib/$rel" ] || fail "usr/$rel is not KWindowSystem's"; kfiles="$kfiles usr/$rel" ;;
            plugins/*|qml/*) [ -e "$Q/$rel" ] || fail "usr/$rel is not Qt's" ;;
            *) fail "usr/$rel: not accounted for" ;;
        esac
    done < <(find "$pkg/usr/plugins" "$pkg/usr/qml" -type f 2>/dev/null)
    qs=$(qt_sources appimage) || fail "cannot read Qt's modules from the pins"
    row Qt "$qt" download.qt.io "$qs" "LGPL-3.0-only (LICENSES/LGPL-3.0.txt, LICENSES/Qt/)" "${qtfiles# } usr/plugins usr/qml"
    if [ -n "$icufiles" ]; then
        icu=$(ls "$Q"/lib/libicuuc.so.*.* | sed -n 's/.*libicuuc\.so\.\([0-9]*\.[0-9]*\)$/\1/p' | head -n 1)
        copy_dir "icu-$icu" ICU "ICU $icu"
        row ICU "$icu" unicode-org/icu "icu4c-${icu//./_}-src.tgz (built by the Qt Company, shipped with Qt on download.qt.io)" "LICENSES/ICU/LICENSE" "${icufiles# }"
    fi
    if [ -n "$kfiles" ]; then
        kf=$(ls "$K"/lib/libKF6WindowSystem.so.*.*.* | sed -n 's/.*\.so\.\([0-9]*\.[0-9]*\.[0-9]*\)$/\1/p' | head -n 1)
        copy_dir "kwindowsystem-$kf/LICENSES" KWindowSystem "KWindowSystem $kf"
        row KWindowSystem "$kf" download.kde.org "kwindowsystem-$kf.tar.xz" "LICENSES/KWindowSystem/" "${kfiles# }"
    fi
    # The AppImage's runtime (its first part, prepended later): the commit it states
    if [ -n "${APPIMAGE_RUNTIME:-}" ]; then
        rc=$(grep -ao 'type2-runtime/commit/[0-9a-f]*' "$APPIMAGE_RUNTIME" | head -n 1 | sed 's|.*/||')
        [ -n "$rc" ] || fail "$APPIMAGE_RUNTIME states no type2-runtime commit"
        # Its licence, squashfuse's and libfuse's (LGPL-2.1), and the sources
        # release-sources.sh fetches for it (SOURCES.txt, from update-licences.sh)
        copy_dir "appimage-runtime-$rc" AppImage-runtime "the AppImage runtime $rc"
        rsrc=$(grep -v '^https://' "$L/AppImage-runtime/SOURCES.txt" | cut -d' ' -f1 | join)   # -- WP LIC1 -- one line (ALPINE.txt adds several)
        for u in $(grep '^https://' "$L/AppImage-runtime/SOURCES.txt"); do
            case $u in https://github.com/vasi/squashfuse/*) rsrc="$rsrc squashfuse-${u##*/}" ;; *) rsrc="$rsrc ${u##*/}" ;; esac
        done
        row "AppImage runtime" "$rc" github.com/AppImage/type2-runtime "$rsrc" \
            "MIT (LICENSES/AppImage-runtime/LICENSE), with libfuse (LGPL-2.1, LICENSES/AppImage-runtime/libfuse-LICENSE and libfuse-LGPL2.txt), squashfuse (LICENSES/AppImage-runtime/squashfuse-LICENSE), and Alpine 3.21's musl (MIT, musl-COPYRIGHT), zlib (Zlib, zlib-LICENSE), zstd (BSD-3-Clause OR GPL-2.0-or-later, zstd-LICENSE and zstd-COPYING) and mimalloc (MIT, mimalloc-LICENSE) linked in, as LICENSES/AppImage-runtime/ALPINE.txt pins them" \
            "the AppImage file's runtime"
    fi
    distro=$(. /etc/os-release && echo "$ID-$VERSION_ID")
    for p in $(sed -n 's/ .*//p' <<<"$debs" | sort -u); do
        files=$(awk -v p="$p" '$1 == p {print "usr/lib/" $2}' <<<"$debs" | join)
        read -r ver src srcver < <(dpkg-query -W -f '${Version} ${source:Package} ${source:Version}\n' "$p")
        mkdir -p "$L/bundled/${p%%:*}"
        cp "/usr/share/doc/${p%%:*}/copyright" "$L/bundled/${p%%:*}/copyright" 2>/dev/null \
            || fail "${p%%:*} has no /usr/share/doc/${p%%:*}/copyright"
        row "${p%%:*}" "$ver" "$distro" "$src=$srcver" "LICENSES/bundled/${p%%:*}/copyright" "$files"
    done
    sources="Their source code, Qt's among it, is included with this release (the source archives and Ubuntu source packages named there)."
    ;;
windows)
    M=${MINGW_ROOT:-/usr/x86_64-w64-mingw32/sys-root/mingw}
    qt=$(rpm -q --qf '%{VERSION}' mingw64-qt6-qtbase)
    copy_dir "qt-$qt" Qt "Qt $qt"
    distro=$(. /etc/os-release && echo "$ID-$VERSION_ID")
    pairs=
    while read -r f; do
        rel=${f#"$pkg"/}
        case $rel in
            DSPiConsole.exe|dspi_capi.dll|qt.conf|NOTICE.txt|LICENSES/*) continue ;;
            qml/*) from=$M/lib/qt6/$rel ;;
            */*) from=$M/lib/qt6/plugins/$rel ;;
            *) from=$M/bin/$rel ;;
        esac
        p=$(rpm -qf --qf '%{NAME}\n' "$from" 2>/dev/null | head -n 1) || true
        [ -n "$p" ] && [ "${p#file }" = "$p" ] || fail "$rel comes from no package of the build image ($from)"
        pairs="$pairs$p $rel"$'\n'
    done < <(find "$pkg" -type f | sort)
    for p in $(sed -n 's/ .*//p' <<<"$pairs" | sort -u); do
        # QML modules as folders, the rest by name
        files=$(awk -v p="$p" '$1 == p {f = $2; if (f ~ /^qml\//) { n = split(f, a, "/"); f = substr(f, 1, length(f) - length(a[n]) - 1) "/" } print f}' <<<"$pairs" | sort -u | join)
        read -r ver srpm < <(rpm -q --qf '%{VERSION}-%{RELEASE} %{SOURCERPM}\n' "$p")
        lic=$(rpm -q --qf '%{LICENSE}' "$p")
        lfiles=$(rpm -qL "$p" | grep -v '^(contains no files)$' || true)
        mkdir -p "$L/bundled/$p"
        if [ -n "$lfiles" ]; then
            for lf in $lfiles; do cp "$lf" "$L/bundled/$p/"; done
        else
            # The package ships no licence file: the one update-licences.sh took from its source RPM
            sname=${srpm%-"$ver".src.rpm}
            fl=$here/fedora/$sname-$(rpm -q --qf '%{VERSION}' "$p")
            [ -d "$fl" ] || fail "$p ships no licence file and $fl is missing (add $sname to update-licences.sh's FEDORA_NO_LICENCE)"
            cp "$fl"/* "$L/bundled/$p/"
        fi
        lic="$lic (LICENSES/bundled/$p/)"
        row "$p" "$ver" "$distro" "$srpm" "$lic" "$files"
    done
    sources="Their source code, Qt's among it, is included with this release (the Fedora source packages named there)."
    ;;
macos)
    qt=$(sed -n 's/^qtver=\([0-9.]*\)$/\1/p' "$here/../macos/fetch-qt.sh")
    built=$("${QTDIR:?set QTDIR, as for build-macos.sh}/bin/qtpaths" --qt-version)
    [ "$built" = "$qt" ] || fail "the app is built on Qt $built, fetch-qt.sh pins $qt: the notice would name the wrong Qt"
    copy_dir "qt-$qt" Qt "Qt $qt"
    fw=$pkg/Contents/Frameworks
    for f in "$fw"/*; do
        [ -e "$f" ] || continue
        case ${f##*/} in Qt*.framework) ;; *) fail "Contents/Frameworks/${f##*/} is not Qt's" ;; esac
    done
    qs=$(qt_sources macos) || fail "cannot read Qt's modules from the pins"
    row Qt "$qt" download.qt.io "$qs" "LGPL-3.0-only (LICENSES/LGPL-3.0.txt, LICENSES/Qt/)" \
        "Contents/Frameworks/Qt*.framework Contents/PlugIns Contents/Resources/qml"
    sources="Qt's source for this version is included with this release (the source archives named there)."
    ;;
swift)
    # -- WP REL013M --
    rm "$L/LGPL-3.0.txt"
    cp "$here/../../../apple/licences/rust-crates.txt" "$L/rust-crates.txt"
    resolved=${PACKAGE_RESOLVED:?set PACKAGE_RESOLVED to Package.resolved of the Xcode project}
    checkouts=${SWIFTPM_CHECKOUTS:?set SWIFTPM_CHECKOUTS to the SwiftPM checkouts in DerivedData}
    # Xcode embeds the Swift runtime's back-deployment libraries (libswift*.dylib)
    # for older macOS versions; nothing else may be there
    rt=
    for f in "$pkg"/Contents/Frameworks/*; do
        [ -e "$f" ] || continue
        case ${f##*/} in
            libswift*.dylib) rt="$rt Contents/Frameworks/${f##*/}" ;;
            *) fail "Contents/Frameworks/${f##*/}: not a Swift runtime library" ;;
        esac
    done
    if [ -n "$rt" ]; then
        sv=$(xcrun swift --version 2>/dev/null | sed -n 's/.*Apple Swift version \([0-9.]*\).*/\1/p' | head -n 1)
        row "Swift runtime" "${sv:-?}" "Xcode $(xcodebuild -version | sed -n 's/^Xcode //p')" github.com/swiftlang/swift \
            "Apache-2.0 with the Runtime Library Exception (https://swift.org/LICENSE.txt)" "${rt# }"
    fi
    i=0
    while id=$(plutil -extract "pins.$i.identity" raw -o - "$resolved" 2>/dev/null); do
        loc=$(plutil -extract "pins.$i.location" raw -o - "$resolved")
        ver=$(plutil -extract "pins.$i.state.version" raw -o - "$resolved")
        rev=$(plutil -extract "pins.$i.state.revision" raw -o - "$resolved")
        [ -d "$checkouts/$id" ] || fail "no checkout of $id in $checkouts"
        mkdir -p "$L/swift/$id"
        for f in "$checkouts/$id"/LICENSE* "$checkouts/$id"/NOTICE*; do
            if [ -f "$f" ]; then cp "$f" "$L/swift/$id/"; fi
        done
        ls "$L/swift/$id"/LICENSE* >/dev/null 2>&1 || fail "$id has no LICENSE file"
        row "$id" "$ver" "${loc%.git}" "$id at $rev" "LICENSES/swift/$id/" "Contents/MacOS/DSPi uConsole (linked in)"
        i=$((i + 1))
    done
    [ "$i" -gt 0 ] || fail "no Swift packages in $resolved"
    sources=
    ;;
*) fail "unknown package kind $kind" ;;
esac

# The notice: plain facts (DRAFT, as TERMS.txt). -- WP CLEAN1 -- The program as a
# whole: GPL-3.0; Bofr5555's own additions: GPL-3.0-or-later with section 7 terms
{
    echo "DSPi uConsole $version"
    echo
    echo "DRAFT notice, pending the owner's approval."
    echo
    echo "DSPi uConsole as a whole is distributed under the GNU General Public License"
    echo "version 3 (LICENSES/GPL-3.0.txt). Bofr5555's own contributions are licensed"
    echo "under version 3 or (at your option) any later version, with additional terms"
    echo "under its section 7 (LICENSES/TERMS.txt, also below)."
    echo "Its complete source code is published with this release, beside its"
    echo "downloads, as $source_asset."
    echo
    echo "The DSPi firmware images it bundles are Weeb Labs' release v1.1.6 (GPL-3.0);"
    echo "their third-party notices are in LICENSES/firmware-THIRD-PARTY-NOTICES.txt."
    echo "Its AutoEQ database holds AutoEq data (MIT, LICENSES/AutoEq-MIT.txt)."
    # -- WP LIC1 -- the Rust crates in its core
    echo
    echo "Its core links third-party Rust crates; they and their licence texts are"
    echo "listed in LICENSES/rust-crates.txt."
    if [ -d "$L/Qt" ]; then   # -- WP REL013M -- not in the Swift app
        echo
        echo "This program uses Qt $qt, licensed under the GNU Lesser General Public"
        echo "License version 3 (LICENSES/LGPL-3.0.txt, which incorporates the GNU General"
        echo "Public License version 3, LICENSES/GPL-3.0.txt). Qt's licence files are in"
        echo "LICENSES/Qt/; the third-party components in Qt and their licences are listed"
        echo "in LICENSES/Qt/THIRD-PARTY.txt."
    fi
    if [ -d "$L/AppImage-runtime" ]; then
        echo
        echo "The AppImage's runtime (AppImage's type2-runtime $rc) contains libfuse,"
        echo "licensed under the GNU Lesser General Public License version 2.1"
        echo "(LICENSES/AppImage-runtime/libfuse-LGPL2.txt), and other code listed in"
        echo "LICENSES/BUNDLED.txt: squashfuse, musl, zlib, zstd and mimalloc, their"
        echo "licence texts in LICENSES/AppImage-runtime/."
    fi
    if [ -d "$L/KWindowSystem" ]; then
        echo
        echo "This program uses KDE's KWindowSystem $kf; its licence files are in"
        echo "LICENSES/KWindowSystem/."
    fi
    echo
    echo "Every library in this package that is not part of DSPi uConsole is listed in"
    echo "LICENSES/BUNDLED.txt, with its version, its source and its licence."
    [ -z "$sources" ] || echo "$sources" | fold -s -w 79 | sed 's/ $//'
    # -- WP LIC1 -- the copyright and the additional terms, verbatim
    echo
    echo "-------------------------------------------------------------------------------"
    echo
    cat "$here/TERMS.txt"
} >"$dest/NOTICE.txt"
echo "stage-licences: $kind: $(grep -vc '^#' "$bundled") bundled libraries listed, NOTICE.txt and LICENSES/ in $dest"
