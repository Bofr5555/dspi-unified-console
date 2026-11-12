#!/usr/bin/env bash
# -- WP QP20 -- Builds the .rpm against Fedora's own Qt in
# Containerfile.fedora's image: the app, checked, then CPack's package
# (qt/cmake/wp/qp20.cmake: License GPL-3.0-only, the udev rule, the notices in
# /usr/share/licenses/dspi-uconsole, the scriptlets). Run from the repository
# root:
#
#   apps/desktop/packaging/linux/build-fedora.sh [output dir, default dist/]
#
# writes dspi-uconsole-<version>-1.fc<N>.x86_64.rpm for the image's Fedora
# release N (Containerfile.fedora says why one per release). A distribution
# package never checks for updates (no DSPI_UPDATE_FEED_URL): dnf does that.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../../.." && pwd)
fc=$(. /etc/os-release && echo "$VERSION_ID")
build=${BUILD_DIR:-$repo/apps/desktop/build-fedora$fc}
out=${1:-$repo/dist}
cd "$repo"
cargo build --release --locked -p dspi-capi --features link,midir,mdns,hub
cmake -S apps/desktop/qt -B "$build" -G Ninja -DCMAKE_BUILD_TYPE=Release \
      -DCMAKE_INSTALL_PREFIX=/usr -DDSPI_CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$repo/target}" \
      -DDSPI_NOTICES=rpm -DDSPI_UPDATE_FEED_URL=
cmake --build "$build" -j"${JOBS:-4}" --target DSPiConsole
# The release's own glibc: the .rpm is for this Fedora release only
"$here/check-binary.sh" "$build/DSPiConsole" "$(ldd --version | sed -n '1s/.* //p')"
(cd "$build" && cpack -G RPM)
mkdir -p "$out"
rpm=$(ls "$build"/dspi-uconsole-*.fc"$fc".x86_64.rpm)
cp "$rpm" "$out/"
rpm -qp --qf 'Name: %{NAME}\nVersion: %{VERSION}-%{RELEASE}\nLicense: %{LICENSE}\n' "$rpm"
ls -la "$out/${rpm##*/}"
