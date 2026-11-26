#!/usr/bin/env bash
# Builds the .deb against Debian's own Qt in Containerfile.debian's image:
# the app, checked, then CPack's package (qt/cmake/wp/qp20.cmake: the
# dependencies, the udev rule, the notices in /usr/share/doc/uconsole-for-dspi,
# the maintainer scripts). Run from the repository root:
#
#   apps/desktop/packaging/linux/build-debian.sh [output dir, default dist/]
#
# -- WP QP20 -- writes uconsole-for-dspi_<version>_amd64.deb. A distribution
# package never checks for updates (no DSPI_UPDATE_FEED_URL): apt does that.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../../.." && pwd)
build=${BUILD_DIR:-$repo/apps/desktop/build-debian}
out=${1:-$repo/dist}
cd "$repo"
cargo build --release --locked -p dspi-capi --features link,midir,mdns,hub
cmake -S apps/desktop/qt -B "$build" -G Ninja -DCMAKE_BUILD_TYPE=Release \
      -DCMAKE_INSTALL_PREFIX=/usr -DDSPI_CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$repo/target}" \
      -DDSPI_NOTICES=deb -DDSPI_UPDATE_FEED_URL=
cmake --build "$build" -j"${JOBS:-4}" --target uconsole-for-dspi
"$here/check-binary.sh" "$build/uconsole-for-dspi" 2.41
# -- WP FIN015 -- the build folder is kept between releases: an older version's package left in it
# would be copied out with this one
rm -f "$build"/uconsole-for-dspi_*_amd64.deb
(cd "$build" && cpack -G DEB)
mkdir -p "$out"
cp "$build"/uconsole-for-dspi_*_amd64.deb "$out/"
dpkg-deb --info "$out"/uconsole-for-dspi_*_amd64.deb | sed -n 's/^ \(Package\|Version\|Depends\|Recommends\):/\1:/p'
ls -la "$out"/uconsole-for-dspi_*_amd64.deb
