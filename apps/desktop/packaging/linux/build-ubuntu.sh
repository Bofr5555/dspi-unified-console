#!/usr/bin/env bash
# Builds the app for the Linux release in Containerfile.ubuntu's image (Qt 6.8
# from download.qt.io in /opt/qt), checks the binary and installs it into an
# AppDir under build-ubuntu/AppDir. Run from the repository root:
#
#   apps/desktop/packaging/linux/build-ubuntu.sh
#
# The AppImage and the archive are made from that AppDir by make-appimage.sh
# (build-release.sh runs both). -- WP QP20 -- This is one of the project's own
# builds, so it carries the update feed (DSPI_UPDATE_FEED_URL; empty: none). The
# AppDir gets no udev rule and no distribution notices (the archive carries the
# rule, stage.sh the notices).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../../.." && pwd)
build=${BUILD_DIR:-$repo/apps/desktop/build-ubuntu}
feed=${DSPI_UPDATE_FEED_URL-https://api.github.com/repos/Bofr5555/dspi-unified-console/releases?per_page=30}
cd "$repo"
cargo build --release --locked -p dspi-capi --features link,midir,mdns,hub
cmake -S apps/desktop/qt -B "$build" -G Ninja -DCMAKE_BUILD_TYPE=Release \
      -DCMAKE_PREFIX_PATH=/opt/qt -DCMAKE_INSTALL_PREFIX=/usr -DDSPI_CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$repo/target}" \
      -DDSPI_NOTICES=none -DDSPI_UDEV_RULES_DIR= \
      -DDSPI_UPDATE_FEED_URL="$feed"
cmake --build "$build" -j"${JOBS:-4}" --target DSPiConsole
"$here/check-binary.sh" "$build/DSPiConsole" 2.39
rm -rf "$build/AppDir"
DESTDIR="$build/AppDir" cmake --install "$build" --strip
echo "build-ubuntu: $build/AppDir"
