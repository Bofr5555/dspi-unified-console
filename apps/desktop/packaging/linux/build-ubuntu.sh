#!/usr/bin/env bash
# Builds the app for the Linux release in Containerfile.ubuntu's image (Qt 6.8
# from download.qt.io in /opt/qt), checks the binary and installs it into an
# AppDir under build-ubuntu/AppDir. Run from the repository root:
#
#   apps/desktop/packaging/linux/build-ubuntu.sh
#
# The AppImage, the archive and their licence texts are made from that AppDir
# by the release packaging (QP20).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../../.." && pwd)
build=${BUILD_DIR:-$repo/apps/desktop/build-ubuntu}
cd "$repo"
cargo build --release --locked -p dspi-capi --features link,midir,mdns,hub
cmake -S apps/desktop/qt -B "$build" -G Ninja -DCMAKE_BUILD_TYPE=Release \
      -DCMAKE_PREFIX_PATH=/opt/qt -DCMAKE_INSTALL_PREFIX=/usr -DDSPI_CARGO_TARGET_DIR="$repo/target"
cmake --build "$build" -j"${JOBS:-4}"
"$here/check-binary.sh" "$build/DSPiConsole" 2.39
rm -rf "$build/AppDir"
DESTDIR="$build/AppDir" cmake --install "$build"
echo "build-ubuntu: $build/AppDir"
