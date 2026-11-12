#!/bin/bash
# -- WP QD42 -- The Qt version and source modules a package bundles, read from
# the pins its build uses (nothing is listed twice):
#   qt-modules.sh appimage   e.g. "6.8.3 qtbase qtdeclarative qtsvg qtwayland"
#                            (linux/Containerfile.ubuntu's manifest, qt-<version>.sha256)
#   qt-modules.sh windows    e.g. "6.11.2 qtbase qtdeclarative qtsvg"
#                            (windows/Containerfile's MINGW_QT_VERSION and packages)
# Each module is the name of its source archive on download.qt.io
# (<module>-everywhere-src-<version>.tar.xz). The Windows build's Qt is Fedora's
# MinGW Qt: its source is Fedora's source packages, listed in the bundle's
# LICENSES/BUNDLED.txt; the version pinned here names the folder of Qt's
# licence texts (licences/qt-<version>/) the bundle gets. -- WP QP20 -- (was
# macos, for the Qt .dmg, which is gone.)
set -euo pipefail
pkg=$(cd "$(dirname "$0")/.." && pwd)
# Pinned for the build only, not in any package: lupdate and lrelease
build_only=" qttools "
case ${1:-} in
    appimage)
        manifest=$pkg/linux/$(grep -o 'qt-[0-9.]*\.sha256' "$pkg/linux/Containerfile.ubuntu" | head -n 1)
        # .../6.8.3-0-202503201345qtwayland-compositor-Linux-...7z: qtwayland's source
        # holds the compositor too; ICU (icu-linux-...) is not Qt's
        paths=$(grep -v '^#' "$manifest" | awk '{print $2}' | grep '\.7z$')
        version=$(sed -n 's|.*/\([0-9]*\.[0-9]*\.[0-9]*\)-0-[0-9]*qt.*|\1|p' <<<"$paths" | sort -u)
        modules=$(sed -n 's|.*/[0-9.]*-0-[0-9]*\(qt[a-z0-9]*\).*|\1|p' <<<"$paths") ;;
    windows)
        f=$pkg/windows/Containerfile
        version=$(sed -n 's/^ARG MINGW_QT_VERSION=\([0-9.]*\)$/\1/p' "$f")
        modules=$(grep -o 'mingw64-qt6-qt[a-z0-9]*' "$f" | sed 's/^mingw64-qt6-//') ;;
    *) echo "usage: qt-modules.sh appimage|windows" >&2; exit 2 ;;
esac
[ "$(wc -w <<<"$version" | tr -d " ")" = 1 ] || { echo "qt-modules: no single Qt version in the pins ('$version')" >&2; exit 1; }
out=$version
for m in $(sort -u <<<"$modules"); do
    case $build_only in *" $m "*) continue ;; esac
    out="$out $m"
done
[ "$out" != "$version" ] || { echo "qt-modules: no Qt modules in the pins" >&2; exit 1; }
echo "$out"
