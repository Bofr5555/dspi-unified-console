#!/bin/bash
# Build the Qt app for the function test: the real app plus the simulated home
# theatre (dspi-core's sim-scenario) and the control socket (DSPI_FUNCTEST), both
# off in every other build. In the build container, into its work folder.
set -euo pipefail
cd "$(dirname "$0")/../.."
exec ci/run.sh "cargo build --release -p dspi-capi --features link,midir,mdns,hub,sim-scenario &&
  cmake -S apps/desktop/qt -B /work/build-functest -G Ninja -DCMAKE_BUILD_TYPE=Release \
        -DDSPI_CARGO_TARGET_DIR=/target -DDSPI_FUNCTEST=ON &&
  cmake --build /work/build-functest -j${DSPI_QT_JOBS:-8} --target uconsole-for-dspi"
