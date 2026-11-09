#!/usr/bin/env bash
# Runs inside the Fedora build container (see build-release.sh): the .rpm, on Qt 6.
set -euo pipefail
here=/src/apps/desktop/packaging/linux
rm -rf /work && mkdir /work
tar -C /src --exclude=./target --exclude=./apps/desktop/qt/build --exclude=./.git --exclude=./dist -cf - . | tar -C /work -xf -
cd /work
cargo build --release --locked -p dspi-capi --features link,midir,mdns,hub
cd /work/apps/desktop
cmake -S qt -B build -G Ninja -DCMAKE_BUILD_TYPE=Release -DDSPI_QT_MAJOR=6
cmake --build build -j"${JOBS:-4}"
"$here/check-binary.sh" build/DSPiConsole 2.41
(cd build && cpack -G RPM && cp dspi-uconsole-*.rpm /dist/)
