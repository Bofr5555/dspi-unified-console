#!/usr/bin/env bash
# Runs inside the Debian build container (see build-release.sh): the .deb,
# against the system Qt 6 (Debian 13 and newer, Ubuntu 25.04 and newer).
set -euo pipefail
here=/src/apps/desktop/packaging/linux

# Build from a copy, leaving the host's build directories alone
rm -rf /work && mkdir /work
tar -C /src --exclude=./target --exclude=./apps/desktop/qt/build --exclude=./.git --exclude=./dist -cf - . | tar -C /work -xf -
cd /work
cargo build --release --locked -p dspi-capi --features link,midir,mdns,hub
cd /work/apps/desktop
cmake -S qt -B build -G Ninja -DCMAKE_BUILD_TYPE=Release -DDSPI_QT_MAJOR=6
cmake --build build -j"${JOBS:-4}"
"$here/check-binary.sh" build/DSPiConsole 2.41
(cd build && cpack -G DEB && cp dspi-console_*.deb /dist/)
