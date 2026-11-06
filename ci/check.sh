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
#       android                       cargo ndk build of dspi-core (arm64, armv7, x86_64)
#       wasm                          dspi-proto for wasm32-unknown-unknown
#       qt                            the Qt 6 desktop app against the C ABI crate
#       proptest                      the fuzz-style property tests, longer run
#   ci/check.sh --all                 full gate plus every optional step
#
# Lint level: clippy's default set, warnings denied (-D warnings). The retired
# crates/dspi-core-linux (package dspi-core-linux) is outside the gates until
# WP Q2 deletes it: it is neither rustfmt- nor clippy-clean, and one of its
# tests compares struct padding.
set -euo pipefail
cd "$(dirname "$0")/.."
RUN=ci/run.sh
EXCLUDE=(--exclude dspi-core-linux)
crates=()
steps=()
while [ $# -gt 0 ]; do
  case "$1" in
    --crate) crates+=("$2"); shift 2 ;;
    --step) steps+=("$2"); shift 2 ;;
    --all) steps=(fmt clippy test windows-gnu windows-msvc android wasm qt proptest); shift ;;
    -h|--help) sed -n '2,24p' "$0"; exit 0 ;;
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
      # Every package but the retired one.
      pk=$($RUN "cargo metadata --no-deps --format-version 1" | python3 -c 'import json,sys; print(" ".join("-p "+p["name"] for p in json.load(sys.stdin)["packages"] if p["name"]!="dspi-core-linux"))')
      $RUN "cargo fmt $pk --check" ;;
    clippy) $RUN "cargo clippy --workspace ${EXCLUDE[*]} --all-features --all-targets -- -D warnings" ;;
    test) $RUN "cargo test --workspace ${EXCLUDE[*]} --all-features --no-fail-fast" ;;
    windows-gnu) $RUN "cargo build --workspace --all-features --target x86_64-pc-windows-gnu" ;;
    windows-msvc)
      DSPI_PROFILE=cross $RUN "cargo xwin build -p dspi-core -p dspi-capi -p dspi-term --features dspi-core/link --target x86_64-pc-windows-msvc" ;;
    android)
      DSPI_PROFILE=cross $RUN "cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 --platform 26 build -p dspi-core --features link" ;;
    wasm) DSPI_PROFILE=cross $RUN "cargo build -p dspi-proto --target wasm32-unknown-unknown" ;;
    qt)
      # Until WP Q2 the app links the old core; WP Q2 switches this to dspi-capi.
      $RUN "cargo build --release -p dspi-core-linux && cmake -S apps/desktop/qt -B /work/build-desktop-qt6 -G Ninja -DCMAKE_BUILD_TYPE=Release -DDSPI_QT_MAJOR=6 -DDSPI_CARGO_TARGET_DIR=/target && cmake --build /work/build-desktop-qt6" ;;
    proptest) $RUN "PROPTEST_CASES=20000 cargo test --workspace ${EXCLUDE[*]} --all-features --release proptest" ;;
    *) echo "unknown step $s" >&2; exit 2 ;;
  esac
  ok "$s"
done
