#!/bin/bash
# The core-merge checks. Runs on the host; every toolchain runs in a container
# (ci/run.sh), never on tmpfs.
#
#   ci/check.sh                       full gate (reviewer/merger): fmt, clippy,
#                                     tests, Windows GNU cross-build
#   ci/check.sh --crate dspi-core     scoped (WP agents): fmt, clippy and tests
#   ci/check.sh --crate a --crate b   of the named packages only
#   ci/check.sh --step NAME ...       one or more optional steps:
#       fmt clippy test               the three gates, whole workspace
#       windows-gnu                   cargo build --workspace, x86_64-pc-windows-gnu
#       windows-msvc                  cargo xwin build of dspi-core, dspi-capi, dspi-term
#       android                       cargo ndk build of dspi-core and dspi-ffi (arm64, armv7, x86_64)
#       wasm                          dspi-proto for wasm32-unknown-unknown
#       qt                            the Qt 6 desktop app (crates/dspi-capi,
#                                     library dspi_capi)
#       qt5                           the same against Qt 5.15
#       qt-offscreen                  the Qt 6 app with its smoke driver, on the
#                                     demo device, offscreen (apps/desktop/qt/smoke)
#       qt-windows                    packaging/windows/build-windows.sh: the
#                                     MinGW Qt 6 app, zip and installer
#       proptest                      the property tests (functions named
#                                     proptest_*), 20000 cases each
#   ci/check.sh --all                 full gate plus every optional step
#
# Lint level: clippy's default set, warnings denied (-D warnings), plus the
# stricter lints the core-merge crates and modules enable themselves
# (unwrap_used, expect_used, indexing_slicing, arithmetic_side_effects;
# clippy.toml exempts test code).
set -euo pipefail
cd "$(dirname "$0")/.."
RUN=ci/run.sh
crates=()
steps=()
while [ $# -gt 0 ]; do
  case "$1" in
    --crate) crates+=("$2"); shift 2 ;;
    --step) steps+=("$2"); shift 2 ;;
    --all) steps=(fmt clippy test windows-gnu windows-msvc android wasm qt qt5 qt-offscreen qt-windows proptest); shift ;;
    -h|--help) sed -n "2,30p" "$0"; exit 0 ;;
    *) echo "unknown argument $1" >&2; exit 2 ;;
  esac
done
if [ ${#crates[@]} -eq 0 ] && [ ${#steps[@]} -eq 0 ]; then
  steps=(fmt clippy test windows-gnu)
fi

ok() { printf '\n== %s: OK\n' "$1"; }

if [ ${#crates[@]} -gt 0 ]; then
  p=(); for c in "${crates[@]}"; do p+=(-p "$c"); done
  $RUN "cargo fmt ${p[*]} --check"; ok "fmt ${crates[*]}"
  $RUN "cargo clippy ${p[*]} --all-features --all-targets -- -D warnings"; ok "clippy ${crates[*]}"
  $RUN "cargo test ${p[*]} --all-features"; ok "test ${crates[*]}"
fi

for s in "${steps[@]}"; do
  case "$s" in
    fmt)
      $RUN "cargo fmt --all --check" ;;
    clippy) $RUN "cargo clippy --workspace --all-features --all-targets -- -D warnings" ;;
    test) $RUN "cargo test --workspace --all-features --no-fail-fast" ;;
    windows-gnu) $RUN "cargo build --workspace --all-features --target x86_64-pc-windows-gnu" ;;
    windows-msvc)
      DSPI_PROFILE=cross $RUN "cargo xwin build -p dspi-core -p dspi-capi -p dspi-term --features dspi-core/link,dspi-core/midir --target x86_64-pc-windows-msvc" ;;
    android)
      DSPI_PROFILE=cross $RUN "cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 --platform 26 build -p dspi-core --features link && cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 --platform 26 build -p dspi-ffi" ;;
    wasm) DSPI_PROFILE=cross $RUN "cargo build -p dspi-proto --target wasm32-unknown-unknown" ;;
    qt)
      # The build directory's name moved with the core: /work/build-desktop-qt6
      # may still cache the old core's paths.
      $RUN "cargo build --release -p dspi-capi && cmake -S apps/desktop/qt -B /work/build-desktop-qt6-capi -G Ninja -DCMAKE_BUILD_TYPE=Release -DDSPI_QT_MAJOR=6 -DDSPI_CARGO_TARGET_DIR=/target && cmake --build /work/build-desktop-qt6-capi" ;;
    qt5)
      $RUN "cargo build --release -p dspi-capi && cmake -S apps/desktop/qt -B /work/build-desktop-qt5 -G Ninja -DCMAKE_BUILD_TYPE=Release -DDSPI_QT_MAJOR=5 -DDSPI_CARGO_TARGET_DIR=/target && cmake --build /work/build-desktop-qt5" ;;
    qt-windows)
      $RUN "BUILD_DIR=/work/build-desktop-win64 apps/desktop/packaging/windows/build-windows.sh /work/dist-windows" ;;
    proptest)
      # Every property test function is named proptest_* (PLAN §1.9).
      $RUN "PROPTEST_CASES=20000 cargo test --workspace --all-features --release -- proptest_" ;;
    *) echo "unknown step $s" >&2; exit 2 ;;
  esac
  ok "$s"
done
