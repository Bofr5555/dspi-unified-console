#!/bin/bash
# The core-merge checks. Runs on the host; every toolchain runs in a container
# (ci/run.sh), never on tmpfs.
#
#   ci/check.sh                       full gate (reviewer/merger): fmt, clippy,
#                                     tests, Windows GNU cross-build, wasm-core,
#                                     Android contract, licences, deny
#   ci/check.sh --crate dspi-core     scoped (WP agents): fmt, clippy and tests
#   ci/check.sh --crate a --crate b   of the named packages only
#   ci/check.sh --step NAME ...       one or more optional steps:
#       fetch                         cargo fetch --locked: the crates for every
#                                     target, which test steps need (WP CG2's
#                                     `cargo tree --offline --target all`)
#       fmt clippy test               the three gates, whole workspace
#       windows-gnu                   cargo build --workspace, x86_64-pc-windows-gnu
#       windows-msvc                  cargo xwin build of dspi-core, dspi-capi, dspi-term
#       android                       cargo ndk build of dspi-core and dspi-ffi (arm64, armv7, x86_64)
#       android-aab                   the Android app's signed release bundle, with a
#                                     throwaway key, in its Gradle container
#                                     (apps/android/tools/ci.sh release)
#       android-tests                 every Android package but release: contract,
#                                     Rust, ktlint, module tests, assemble, in the
#                                     Gradle container (apps/android/tools/ci.sh tests)
#       android-contract              the Android contract matches crates/
#                                     (gen_kotlin_contract.py --check)
#       wasm                          dspi-proto for wasm32-unknown-unknown
#       wasm-core                     dspi-session and dspi-core for wasm32-unknown-
#                                     unknown, without `usb`, also with `link`; their
#                                     library code and dspi-link's may not call std's
#                                     clock or sleep (ci/wasm-clippy, use
#                                     dspi_transport::clock). Then runs the core there
#                                     (WP WB2, ci/wasm-scenario/run.sh): Node with
#                                     JSPI (NODE in images.conf), the demo device in
#                                     two sessions, no threads
#       qt                            the Qt 6 desktop app (crates/dspi-capi,
#                                     library dspi_capi)
#       qt5                           the same against Qt 5.15
#       qt-ts                         translations/dspi-console_en.ts is what
#                                     lupdate makes of the sources (WP QD40; the
#                                     qt step runs it too)
#       qt-offscreen                  the Qt 6 app with its smoke driver, on the
#                                     demo device, offscreen (apps/desktop/qt/smoke)
#       qt5-offscreen                 the same on Qt 5.15
#       terminal                      the Linux terminal binary (dspi-term), run once
#       qt-windows                    packaging/windows/build-windows.sh: the
#                                     MinGW Qt 6 app, zip and installer
#       proptest                      the property tests (functions named
#                                     proptest_*), 20000 cases each
#       kotlin                        generate the UniFFI bindings and compile
#                                     them with kotlinc (ci/compile-bindings.sh)
#       swift                         the same with swiftc, linked to the library
#       licences                      the iOS build against docs/licensing/
#                                     (ci/licence-inventory.py --gate); a new
#                                     crate, licence, asset or outside author fails.
#                                     Runs python3 (stdlib only) and git on the host,
#                                     cargo in the container; may clone the Console
#                                     fork on first use
#       drift                         regenerate the C header, the Kotlin and Swift
#                                     bindings and the Android contract; any
#                                     difference from what is committed fails
#                                     (ci/drift.sh)
#       deny                          cargo-deny: licences and sources, whole
#                                     workspace (deny.toml)
#       ios-check                     dspi-ffi without `usb`: nusb is out of the iOS
#                                     graph, clippy and tests pass with the feature
#                                     off, and `cargo check` for aarch64-apple-ios
#                                     (no link: no Apple SDK here; linking is SW1
#                                     in macvm). Fails if the image has no iOS std
#       conformance-ts                upstream's TypeScript Link client against the
#                                     hub (crates/dspi-hub/tests/conformance/
#                                     run-clients.sh ts): the build container and
#                                     NODE from images.conf; needs the DSPi-Link
#                                     clone in work/consoles (README there)
#       web                           the web client (apps/web/tools/ci.sh): npm ci
#                                     from the lockfile, typecheck, unit tests, and
#                                     e2e in Chromium and Firefox on the demo
#                                     device, in PLAYWRIGHT from images.conf
#   ci/check.sh --all                 full gate plus every optional step
#   ci/check.sh --all-clients         every client, then the matrix (ci/matrix.sh);
#                                     builds missing images first
#
# Lint level: clippy's default set, warnings denied (-D warnings), plus the
# stricter lints the core-merge crates and modules enable themselves
# (unwrap_used, expect_used, indexing_slicing, arithmetic_side_effects;
# clippy.toml exempts test code).
set -euo pipefail
cd "$(dirname "$0")/.."
RUN=ci/run.sh
# -- WP MR2 -- The tags of every image (DENY below), and the features the Qt app's
# library needs for CG1's and CG2's code to exist in it.
# shellcheck source=container/images.conf
. ci/container/images.conf
# -- WP HB6 -- hub: the smoke driver starts one (SmokeDriverHub.cpp)
CAPI_FEATURES=link,midir,mdns,hub
# -- WP QD8 -- The smoke driver's Link test talks to this binary.
FAKEHUB="cargo build --release -p dspi-link --features fake-hub --bin dspi-link-fakehub"
# -- WP HB6 -- The Terminal, as the smoke driver's client of a demo hub
TERMINAL="cargo build --release --locked -p dspi-term"
# -- WP QD40 -- The Qt app's .ts is what lupdate makes of the sources now: lupdate
# runs as the dspi_lupdate target does (apps/desktop/qt/CMakeLists.txt, the same
# arguments) on a copy of qml/, src/ and translations/ laid out as in the tree,
# and any difference from the committed file fails. -- WP QD41 -- Without
# locations (-locations none), so moved lines don't make a difference; the fix
# names the command, which runs in the CI image (lupdate 6.11.2: another version
# may write the file differently).
TSUPDATE='lupdate-qt6 -silent -locations none -no-obsolete -source-language en -target-language en qml src -ts translations/dspi-console_en.ts'
TSCHECK='set -e; d=/work/ts-check; rm -rf "$d"; mkdir -p "$d"
  cp -r apps/desktop/qt/qml apps/desktop/qt/src apps/desktop/qt/translations "$d"/
  (cd "$d" && '"$TSUPDATE"')
  if ! diff -u apps/desktop/qt/translations/dspi-console_en.ts "$d"/translations/dspi-console_en.ts; then
    echo "qt-ts: apps/desktop/qt/translations/dspi-console_en.ts is not current. Regenerate it in the CI image (lupdate $(lupdate-qt6 -version 2>&1 | grep -o "[0-9][0-9.]*" | head -1)) and commit it:" >&2
    echo "  ci/run.sh '"'"'cd apps/desktop/qt && '"$TSUPDATE"''"'"'" >&2
    exit 1
  fi
  echo "qt-ts: dspi-console_en.ts is current"'
# -- WP QD40 end --
# -- WP CG9 -- The image with kotlinc and swiftc. Other steps keep ci/run.sh's.
crates=()
steps=()
while [ $# -gt 0 ]; do
  case "$1" in
    --crate) crates+=("$2"); shift 2 ;;
    --step) steps+=("$2"); shift 2 ;;
    --all) steps=(fetch fmt clippy test windows-gnu android-contract licences deny windows-msvc android android-tests android-aab wasm wasm-core qt qt5 qt-offscreen qt5-offscreen terminal qt-windows proptest kotlin swift ios-check drift conformance-ts web); shift ;;
    --all-clients) exec ci/matrix.sh ;;
    -h|--help) sed -n "2,/^set -euo/p" "$0" | sed '$d'; exit 0 ;;
    *) echo "unknown argument $1" >&2; exit 2 ;;
  esac
done
if [ ${#crates[@]} -eq 0 ] && [ ${#steps[@]} -eq 0 ]; then
  # -- WP WB1 -- wasm-core
  # -- WP MR3 -- android-contract
  steps=(fmt clippy test windows-gnu wasm-core android-contract licences deny)
fi

ok() { printf '\n== %s: OK\n' "$1"; }

if [ ${#crates[@]} -gt 0 ]; then
  p=(); for c in "${crates[@]}"; do p+=(-p "$c"); done
  $RUN "cargo fmt ${p[*]} --check"; ok "fmt ${crates[*]}"
  $RUN "cargo clippy ${p[*]} --all-features --all-targets -- -D warnings"; ok "clippy ${crates[*]}"
  $RUN "cargo fetch --locked && cargo test ${p[*]} --all-features"; ok "test ${crates[*]}"
fi

for s in "${steps[@]}"; do
  case "$s" in
    fetch) $RUN "cargo fetch --locked" ;;
    fmt)
      $RUN "cargo fmt --all --check" ;;
    clippy) $RUN "cargo clippy --workspace --all-features --all-targets -- -D warnings" ;;
    test) $RUN "cargo fetch --locked && cargo test --workspace --all-features --no-fail-fast" ;;
    windows-gnu) $RUN "cargo build --workspace --all-features --target x86_64-pc-windows-gnu" ;;
    windows-msvc)
      DSPI_PROFILE=cross $RUN "cargo xwin build -p dspi-core -p dspi-capi -p dspi-term --features dspi-core/link,dspi-core/midir,dspi-capi/link,dspi-capi/midir,dspi-capi/mdns --target x86_64-pc-windows-msvc" ;;
    android-aab)
      # -- WP MR2 -- Since AN1 the app builds on crates/ by path: no submodule.
      apps/android/tools/ci.sh release ;;
    android-tests)
      # -- WP MR3 -- WriteGuardedCore's classification test (ReadOnlyTest) and the like.
      apps/android/tools/ci.sh tests ;;
    android-contract)
      # -- WP MR3 --
      $RUN "cd apps/android && python3 tools/gen_kotlin_contract.py --check" ;;
    android)
      DSPI_PROFILE=cross $RUN "cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 --platform 26 build -p dspi-core --features link && cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 --platform 26 build -p dspi-ffi" ;;
    wasm) DSPI_PROFILE=cross $RUN "cargo build -p dspi-proto --target wasm32-unknown-unknown" ;;
    wasm-core)
      # -- WP WB1 -- dspi-session twice: with every default but `usb`, and as dspi-core
      # uses it. Only --lib is linted, so tests keep std's clock.
      DSPI_PROFILE=cross $RUN "set -e
        t='--target wasm32-unknown-unknown --locked'
        cargo check -p dspi-session --no-default-features --features serde,bundled-autoeq,undo \$t
        cargo check -p dspi-session -p dspi-core --no-default-features \$t
        CLIPPY_CONF_DIR=ci/wasm-clippy cargo clippy -p dspi-transport -p dspi-session -p dspi-core \\
          --no-default-features --lib \$t -- -A clippy::all -D clippy::disallowed_methods -D warnings
        # -- WP WB2 -- with Link: dspi-link's clock and getrandom's backend
        CLIPPY_CONF_DIR=ci/wasm-clippy cargo clippy -p dspi-link -p dspi-core \\
          --no-default-features --features dspi-core/link --lib \$t -- -A clippy::all -D clippy::disallowed_methods -D warnings
        # -- WP WB3 -- the binding, its wasm-only exports included
        CLIPPY_CONF_DIR=ci/wasm-clippy cargo clippy -p dspi-wasm --all-features --lib \$t -- -D clippy::disallowed_methods -D warnings"
      # -- WP WB2 -- and run it
      ci/wasm-scenario/run.sh ;;
    qt)
      # The build directory's name moved with the core: /work/build-desktop-qt6
      # may still cache the old core's paths.
      $RUN "cargo build --release -p dspi-capi --features $CAPI_FEATURES && cmake -S apps/desktop/qt -B /work/build-desktop-qt6-capi -G Ninja -DCMAKE_BUILD_TYPE=Release -DDSPI_QT_MAJOR=6 -DDSPI_CARGO_TARGET_DIR=/target && cmake --build /work/build-desktop-qt6-capi"
      # -- WP QD40 -- and the .ts is current
      $RUN "$TSCHECK" ;;
    qt-ts)
      # -- WP QD40 -- only the .ts check (the qt step runs it too)
      $RUN "$TSCHECK" ;;
    qt5)
      $RUN "cargo build --release -p dspi-capi --features $CAPI_FEATURES && cmake -S apps/desktop/qt -B /work/build-desktop-qt5 -G Ninja -DCMAKE_BUILD_TYPE=Release -DDSPI_QT_MAJOR=5 -DDSPI_CARGO_TARGET_DIR=/target && cmake --build /work/build-desktop-qt5" ;;
    qt-offscreen)
      $RUN "cargo build --release -p dspi-capi --features $CAPI_FEATURES && $FAKEHUB && $TERMINAL && cmake -S apps/desktop/qt -B /work/build-desktop-qt6-smoke -G Ninja -DCMAKE_BUILD_TYPE=Release -DDSPI_QT_MAJOR=6 -DDSPI_CARGO_TARGET_DIR=/target -DDSPI_SMOKE=ON && cmake --build /work/build-desktop-qt6-smoke --target DSPiConsoleSmoke && DSPI_SMOKE_FAKEHUB=/target/release/dspi-link-fakehub DSPI_SMOKE_TERMINAL=/target/release/dspi apps/desktop/qt/smoke/run.sh /work/build-desktop-qt6-smoke/DSPiConsoleSmoke /work/smoke-out && DSPI_SMOKE_HW=dry apps/desktop/qt/smoke/run.sh /work/build-desktop-qt6-smoke/DSPiConsoleSmoke /work/smoke-out-hw-dry" ;;
    qt5-offscreen)
      $RUN "cargo build --release -p dspi-capi --features $CAPI_FEATURES && $FAKEHUB && $TERMINAL && cmake -S apps/desktop/qt -B /work/build-desktop-qt5-smoke -G Ninja -DCMAKE_BUILD_TYPE=Release -DDSPI_QT_MAJOR=5 -DDSPI_CARGO_TARGET_DIR=/target -DDSPI_SMOKE=ON && cmake --build /work/build-desktop-qt5-smoke --target DSPiConsoleSmoke && DSPI_SMOKE_FAKEHUB=/target/release/dspi-link-fakehub DSPI_SMOKE_TERMINAL=/target/release/dspi apps/desktop/qt/smoke/run.sh /work/build-desktop-qt5-smoke/DSPiConsoleSmoke /work/smoke-out-qt5" ;;
    qt-windows)
      $RUN "BUILD_DIR=/work/build-desktop-win64 apps/desktop/packaging/windows/build-windows.sh /work/dist-windows" ;;
    proptest)
      # Every property test function is named proptest_* (PLAN §1.9).
      $RUN "PROPTEST_CASES=20000 cargo test --workspace --all-features --release -- proptest_" ;;
    terminal)
      # -- WP MR2 --
      $RUN "cargo build --release --locked -p dspi-term && /target/release/dspi --version" ;;
    kotlin|swift)
      $RUN "ci/compile-bindings.sh $s" ;;
    licences) python3 ci/licence-inventory.py --gate --fetch-console ;;
    deny)
      podman image exists "$DENY" || ci/container/build-images.sh
      DSPI_IMAGE=$DENY $RUN "cargo deny --workspace --all-features --locked check licenses bans sources" ;;
    drift)
      # -- WP MR2 --
      $RUN "ci/drift.sh check" ;;
    conformance-ts)
      # -- WP HB8 --
      crates/dspi-hub/tests/conformance/run-clients.sh ts ;;
    web)
      # -- WP WB5 --
      apps/web/tools/ci.sh ;;
    ios-check)
      # -- WP CG3 --
      $RUN 'set -euo pipefail
        cargo fetch --locked
        ios="-p dspi-ffi --no-default-features --target aarch64-apple-ios --locked"
        if cargo tree $ios -e normal --prefix none | grep -E "^nusb "; then
          echo "ios-check: nusb is in the iOS graph" >&2; exit 1
        fi
        echo "ios-check: nusb is not in the iOS graph"
        cargo clippy -p dspi-transport -p dspi-session --no-default-features --all-targets --locked -- -D warnings
        cargo clippy -p dspi-core --no-default-features --all-targets --locked -- -D warnings
        cargo clippy -p dspi-core --no-default-features --features uniffi,link --all-targets --locked -- -D warnings
        cargo clippy -p dspi-ffi --no-default-features --lib --tests --locked -- -D warnings
        cargo test -p dspi-transport -p dspi-session --no-default-features --locked
        cargo test -p dspi-core --no-default-features --features uniffi,link --locked
        cargo test -p dspi-ffi --no-default-features --lib --locked
        if ! rustup target list --installed | grep -qx aarch64-apple-ios; then
          echo "ios-check: NOT RUN: this image has no rust-std for aarch64-apple-ios (rustup target add in ci/container/Containerfile)" >&2
          exit 3
        fi
        cargo check $ios --lib' ;;
    *) echo "unknown step $s" >&2; exit 2 ;;
  esac
  ok "$s"
done
