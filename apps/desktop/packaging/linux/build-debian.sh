#!/usr/bin/env bash
# Builds the app against Debian's own Qt in Containerfile.debian's image, checks
# the binary and installs it into build-debian/root, the tree the .deb is made
# from (the .deb itself is the release packaging's, QP20). Run from the
# repository root:
#
#   apps/desktop/packaging/linux/build-debian.sh
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../../.." && pwd)
build=${BUILD_DIR:-$repo/apps/desktop/build-debian}
cd "$repo"
cargo build --release --locked -p dspi-capi --features link,midir,mdns,hub
cmake -S apps/desktop/qt -B "$build" -G Ninja -DCMAKE_BUILD_TYPE=Release \
      -DCMAKE_INSTALL_PREFIX=/usr -DDSPI_CARGO_TARGET_DIR="$repo/target"
cmake --build "$build" -j"${JOBS:-4}"
"$here/check-binary.sh" "$build/DSPiConsole" 2.41
rm -rf "$build/root"
DESTDIR="$build/root" cmake --install "$build"
echo "build-debian: $build/root"
