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
#       windows-msvc                  cargo xwin build of dspi-core, dspi-capi, dspi-term (uconsole)
#       android                       cargo ndk build of dspi-core and dspi-ffi (arm64, armv7, x86_64)
#       android-aab                   the Android app's signed release bundle, with a
#                                     throwaway key, in its Gradle container
#                                     (apps/android/tools/ci.sh release)
#       android-tests                 every Android package but release: contract,
#                                     Rust, ktlint, module tests, assemble, in the
#                                     Gradle container (apps/android/tools/ci.sh tests)
#       android-contract              the Android contract matches crates/
#                                     (gen_kotlin_contract.py --check), and no emoji
#                                     in the app's strings (tools/check_glyphs.py)
#       wasm                          dspi-proto for wasm32-unknown-unknown
#       wasm-core                     dspi-session and dspi-core for wasm32-unknown-
#                                     unknown, without `usb`, also with `link`; their
#                                     library code and dspi-link's may not call std's
#                                     clock or sleep (ci/wasm-clippy, use
#                                     dspi_transport::clock). Then runs the core there
#                                     (WP WB2, ci/wasm-scenario/run.sh): Node with
#                                     JSPI (NODE in images.conf), the demo device in
#                                     two sessions, no threads
#       qt                            the Qt desktop app, Qt 6.8 or newer
#                                     (crates/dspi-capi, library dspi_capi); the
#                                     .ts check only on qtport/main or with
#                                     DSPI_TS_CHECK=1; the theme's contrast
#       qt-ts                         translations/uconsole-for-dspi_en.ts is what
#                                     lupdate makes of the sources (WP QD40; the
#                                     qt step runs it too)
#       qt-offscreen                  the Qt app with its smoke driver, on the
#                                     demo device, offscreen (apps/desktop/qt/smoke)
#       terminal                      the Linux terminal binary (dspi-term: uconsole), run once
#       qt-windows                    packaging/windows/build-windows.sh: the
#                                     MinGW Qt 6 app, zip and installer; then
#                                     test-installer.sh runs them under Wine
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
#                                     workspace (deny.toml); -- WP SECDENY -- and
#                                     RustSec advisories, the database fetched in
#                                     the container (cached copy when offline)
#       firmware-digests              -- WP SECFWV -- the Android, Qt and Apple
#                                     bundles' .uf2 files against the digests the
#                                     Android supply-chain test pins, and
#                                     firmware/manifest.json current
#                                     (tools/firmware_manifest.py --check, with its
#                                     own tests). python3 stdlib on the host
#       ios-check                     dspi-ffi without `usb`: nusb is out of the iOS
#                                     graph, clippy and tests pass with the feature
#                                     off. Fails if the image has no iOS std.
#                                     -- WP SECCORE -- no `cargo check` for
#                                     aarch64-apple-ios here: ring needs the Apple
#                                     SDK; ci/macvm.sh ios compiles it
#       conformance-ts                upstream's TypeScript Link client against the
#                                     hub (crates/dspi-hub/tests/conformance/
#                                     run-clients.sh ts): the build container and
#                                     NODE from images.conf; needs the DSPi-Link
#                                     clone in work/consoles (README there)
#       web                           the web client (apps/web/tools/ci.sh): npm ci
#                                     from the lockfile, typecheck, unit tests, and
#                                     e2e in Chromium and Firefox on the demo
#                                     device, in PLAYWRIGHT from images.conf
#       esp-rig                       NOT in --all (WP LK3): the emulated ESP32
#                                     DSPi-Link bridge (tools/esp-rig/ci.sh): upstream's
#                                     firmware in Espressif's QEMU, the demo device on
#                                     its UART; our client and the TS client must reach
#                                     live meters through it
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
# -- WP QD8 -- The smoke driver's Link test talks to this binary. -- WP QP0 -- Unused
# until the new tree's Link and hub sections come (QP13, QP14). -- WP QP13 -- Built
# by the qt-offscreen step again.
FAKEHUB="cargo build --release -p dspi-link --features fake-hub --bin dspi-link-fakehub"
# -- WP HB6 -- The Terminal, as the smoke driver's client of a demo hub
TERMINAL="cargo build --release --locked -p dspi-term"
# -- WP CLEAN1 -- The Terminal's command name (crates/dspi-cmd's command!())
CMD=$(ci/command-name.sh)
# -- WP QD40 -- The Qt app's .ts is what lupdate makes of the sources now: lupdate
# runs as the dspi_lupdate target does (apps/desktop/qt/CMakeLists.txt, the same
# arguments) on a copy of qml/, src/ and translations/ laid out as in the tree,
# and any difference from the committed file fails. -- WP QD41 -- Without
# locations (-locations none), so moved lines don't make a difference; the fix
# names the command, which runs in the CI image (lupdate 6.11.2: another version
# may write the file differently).
# -- WP QP0 -- The new tree's file is uconsole-for-dspi_en.ts.
TSUPDATE='lupdate-qt6 -silent -locations none -no-obsolete -source-language en -target-language en qml src -ts translations/uconsole-for-dspi_en.ts'
TSCHECK='set -e; d=/work/ts-check; rm -rf "$d"; mkdir -p "$d"
  cp -r apps/desktop/qt/qml apps/desktop/qt/src apps/desktop/qt/translations "$d"/
  (cd "$d" && '"$TSUPDATE"')
  if ! diff -u apps/desktop/qt/translations/uconsole-for-dspi_en.ts "$d"/translations/uconsole-for-dspi_en.ts; then
    echo "qt-ts: apps/desktop/qt/translations/uconsole-for-dspi_en.ts is not current. Regenerate it in the CI image (lupdate $(lupdate-qt6 -version 2>&1 | grep -o "[0-9][0-9.]*" | head -1)) and commit it:" >&2
    echo "  ci/run.sh '"'"'cd apps/desktop/qt && '"$TSUPDATE"''"'"'" >&2
    exit 1
  fi
  echo "qt-ts: uconsole-for-dspi_en.ts is current"'
# -- WP QD40 end --
# -- WP CG9 -- The image with kotlinc and swiftc. Other steps keep ci/run.sh's.
crates=()
steps=()
while [ $# -gt 0 ]; do
  case "$1" in
    --crate) crates+=("$2"); shift 2 ;;
    --step) steps+=("$2"); shift 2 ;;
    --all) steps=(fetch fmt clippy test windows-gnu android-contract licences deny firmware-digests windows-msvc android android-tests android-aab wasm wasm-core qt qt-offscreen qt-windows terminal proptest kotlin swift ios-check drift conformance-ts web); shift ;;
    --all-clients) exec ci/matrix.sh ;;
    -h|--help) sed -n "2,/^set -euo/p" "$0" | sed '$d'; exit 0 ;;
    *) echo "unknown argument $1" >&2; exit 2 ;;
  esac
done
if [ ${#crates[@]} -eq 0 ] && [ ${#steps[@]} -eq 0 ]; then
  # -- WP WB1 -- wasm-core
  # -- WP MR3 -- android-contract
  # -- WP SECFWV -- firmware-digests
  steps=(fmt clippy test windows-gnu wasm-core android-contract licences deny firmware-digests)
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
      $RUN "cd apps/android && python3 tools/gen_kotlin_contract.py --check"
      # -- WP P1EFIX2 -- no emoji or pictographic glyphs in the app's strings
      $RUN "cd apps/android && python3 tools/check_glyphs.py" ;;
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
      # -- WP QP0 -- The new tree (Qt 6.8 or newer, no Qt 5); its own build directory
      $RUN "cargo build --release -p dspi-capi --features $CAPI_FEATURES && cmake -S apps/desktop/qt -B /work/build-desktop-qt -G Ninja -DCMAKE_BUILD_TYPE=Release -DDSPI_CARGO_TARGET_DIR=/target && cmake --build /work/build-desktop-qt"
      # -- WP QD40 -- and the .ts is current. -- WP QP0 -- Only on the integration
      # branches (or DSPI_TS_CHECK=1): a work package's branch does not commit the
      # .ts, the coordinator regenerates it when merging (apps/desktop/README.md)
      case "$(git rev-parse --abbrev-ref HEAD 2>/dev/null)" in
        qtport|main) ts_check=1 ;;
        *) ts_check=${DSPI_TS_CHECK:-0} ;;
      esac
      if [ "$ts_check" = 1 ]; then $RUN "$TSCHECK"
      else echo "qt: the .ts check is skipped on this branch (the coordinator runs --step qt-ts after merging)"; fi
      # -- WP QP0 -- the theme's text colours reach WCAG AA
      $RUN "python3 apps/desktop/qt/tools/contrast.py apps/desktop/qt/qml/Theme/Theme.qml apps/desktop/qt/qml/Theme/ChannelPalette.qml"
      # -- WP LIC1 -- and the packages' Rust crates notice is current
      $RUN "python3 apps/desktop/packaging/licences/gen_rust_notices.py --check" ;;
    qt-ts)
      # -- WP QD40 -- only the .ts check (the qt step runs it too)
      $RUN "$TSCHECK" ;;
    qt-offscreen)
      # -- WP QP0 -- The smoke driver's sections on the demo device; run.sh then
      # runs its other launches (demo entry, tray, pseudo locale, the hardware
      # dry run, QP19). DSPI_SMOKE_SECTION=<name> runs one section.
      # -- WP QP13 -- The fake hub is back, for the Link sections (DSPI_SMOKE_FAKEHUB).
      # -- WP LKUI1 -- And the Terminal, whose `--demo hub --alink` is the
      # System window's hub (DSPI_SMOKE_UCONSOLE).
      $RUN "cargo build --release -p dspi-capi --features $CAPI_FEATURES && $FAKEHUB && $TERMINAL && cmake -S apps/desktop/qt -B /work/build-desktop-qt-smoke -G Ninja -DCMAKE_BUILD_TYPE=Release -DDSPI_CARGO_TARGET_DIR=/target -DDSPI_SMOKE=ON && cmake --build /work/build-desktop-qt-smoke --target uconsole-for-dspi-smoke && DSPI_SMOKE_FAKEHUB=/target/release/dspi-link-fakehub DSPI_SMOKE_UCONSOLE=/target/release/$CMD DSPI_SMOKE_SECTION=${DSPI_SMOKE_SECTION:-} apps/desktop/qt/smoke/run.sh /work/build-desktop-qt-smoke/uconsole-for-dspi-smoke /work/smoke-out" ;;
    qt-windows)
      # -- WP QP20 -- Back on: the zip and the installer, then the installer's
      # test under Wine (install over an earlier install, refused while the app
      # runs, the notices, uninstall). The update feed is left out here: a CI
      # build is not a release.
      $RUN "BUILD_DIR=/work/build-desktop-win64 DSPI_UPDATE_FEED_URL= apps/desktop/packaging/windows/build-windows.sh /work/dist-windows && apps/desktop/packaging/windows/test-installer.sh /work/dist-windows/uConsole-for-DSPi-*-windows-x86_64-setup.exe" ;;
    proptest)
      # Every property test function is named proptest_* (PLAN §1.9).
      $RUN "PROPTEST_CASES=20000 cargo test --workspace --all-features --release -- proptest_" ;;
    terminal)
      # -- WP MR2 --
      $RUN "cargo build --release --locked -p dspi-term && /target/release/$CMD --version && python3 apps/terminal/tools/gen_rust_notices.py --check" ;;
    kotlin|swift)
      $RUN "ci/compile-bindings.sh $s"
      # -- WP REL013M -- and the Swift macOS app's Rust crates notice is current
      if [ "$s" = swift ]; then $RUN "python3 apps/desktop/packaging/licences/gen_rust_notices.py --swift --check"; fi
      # -- WP SW-M1 -- the core's raw USB pipe (no lock, no token) stays inside
      # Core/CoreUsb.swift, the hub's device; its test mock is the one other user
      if [ "$s" = swift ]; then
        stray=$(grep -rlE '\b(UsbPipe|UsbPipeProtocol)\b' --include='*.swift' apps/apple |
          grep -vE '/Core/Generated/|/Core/CoreUsb\.swift$|/DSPi ConsoleTests/CoreUsbSWM1Tests\.swift$' || true)
        if [ -n "$stray" ]; then echo "swift: the raw USB pipe is used outside Core/CoreUsb.swift:" >&2; echo "$stray" >&2; exit 1; fi
        # -- WP SW-M3 -- the bus listing and hot-plug wait (no transfers) also
        # in the core's own USB link (CoreBackedTransport's Link.usb)
        stray=$(grep -rlE '\b(usbPipeScan|usbPipeWaitChange)\b' --include='*.swift' apps/apple |
          grep -vE '/Core/Generated/|/Core/CoreUsb\.swift$|/Core/CoreBackedTransport\.swift$|/DSPi ConsoleTests/CoreUsbSWM1Tests\.swift$' || true)
        if [ -n "$stray" ]; then echo "swift: the USB bus listing is used outside Core/CoreUsb.swift and Core/CoreBackedTransport.swift:" >&2; echo "$stray" >&2; exit 1; fi
        echo "swift: the raw USB pipe is only in Core/CoreUsb.swift"
        # -- WP SW-M3 -- the Rust hub (module dspi_hub) is driven only by
        # Core/CoreHub.swift; the Swift hub (LinkHub, LinkServer, LinkService) is
        # built only by AppState (the opt-out); tests excepted
        stray=$( { grep -rlE '^import dspi_hub$|\bHubHost\(' --include='*.swift' apps/apple | grep -vE '/Core/Generated/|/Core/CoreHub\.swift$|/DSPi ConsoleTests/'
                   grep -rlE '\b(LinkHub|LinkServer|LinkService)\(' --include='*.swift' apps/apple | grep -vE '/DSPi Console/DSPi_ConsoleApp\.swift$|/DSPi ConsoleTests/'; } || true)
        if [ -n "$stray" ]; then echo "swift: the Rust hub is driven outside Core/CoreHub.swift, or the Swift hub is built outside the opt-out:" >&2; echo "$stray" >&2; exit 1; fi
        echo "swift: the Rust hub is only in Core/CoreHub.swift; the Swift hub only in the opt-out"
        # -- WP SW5 -- the core's MIDI handle stays in MIDI/CoreMidiPipe.swift,
        # and the Swift MIDI engine (the opt-out) is built only by AppState's
        # opt-out branches and by tests. -- WP SW5F -- the core's call is
        # openMidi(input:), not a Swift method of the same name (review sw5 F3)
        stray=$(grep -rlE '\b(MidiInputHandle|MidiSender)\b|\bopenMidi\(input:|\.closePort\(|withSender\(' --include='*.swift' apps/apple |
          grep -vE '/Core/Generated/|/MIDI/CoreMidiPipe\.swift$' || true)
        if [ -n "$stray" ]; then echo "swift: the core's MIDI handle is used outside MIDI/CoreMidiPipe.swift:" >&2; echo "$stray" >&2; exit 1; fi
        stray=$(grep -rlE '\bLegacyMIDITransport\(' --include='*.swift' apps/apple |
          grep -vE '/DSPi Console/DSPi_ConsoleApp\.swift$|/DSPi ConsoleTests/' || true)
        if [ -n "$stray" ]; then echo "swift: the Swift MIDI engine is built outside the opt-out:" >&2; echo "$stray" >&2; exit 1; fi
        echo "swift: the core's MIDI handle is only in MIDI/CoreMidiPipe.swift; the Swift MIDI engine only in the opt-out"
        # -- WP SW6 -- the core's DSPi Link session is driven only by
        # Core/CoreLink.swift (and its device bind by CoreBackedTransport); Swift's
        # own Link client (the opt-out) is built only by AppState's opt-out
        # branches, NetworkTransport only by the composite's opt-out factory, and
        # a raw WebSocket only by that client; tests excepted
        stray=$(grep -rlE '\.(linkConnect|linkPair|linkSelectDevice|linkClose|linkRetryNow|linkSetUrl|setLinkListener)\(|\bLinkListener\b' --include='*.swift' apps/apple |
          grep -vE '/Core/Generated/|/Core/CoreLink\.swift$|/Core/CoreBackedTransport\.swift$|/DSPi ConsoleTests/|/DSPi Console iOSUITests/' || true)
        if [ -n "$stray" ]; then echo "swift: the core's Link session is driven outside Core/CoreLink.swift:" >&2; echo "$stray" >&2; exit 1; fi
        stray=$( { grep -rlE '\bLinkClient\(\)' --include='*.swift' apps/apple | grep -vE '/DSPi Console/DSPi_ConsoleApp\.swift$|/DSPi ConsoleTests/|/DSPi Console iOSUITests/'
                   grep -rlE '\bNetworkTransport\(' --include='*.swift' apps/apple | grep -vE '/Link/CompositeTransport\.swift$|/DSPi ConsoleTests/|/DSPi Console iOSUITests/'
                   grep -rlE 'webSocketTask\(' --include='*.swift' apps/apple | grep -vE '/Link/LinkClient\.swift$|/DSPi ConsoleTests/|/DSPi Console iOSUITests/'; } || true)
        if [ -n "$stray" ]; then echo "swift: Swift's own Link client is built outside the opt-out:" >&2; echo "$stray" >&2; exit 1; fi
        echo "swift: the core's Link session is only in Core/CoreLink.swift; Swift's Link client only in the opt-out"
        # -- WP SW-M2 -- firmware reaches a bootloader drive only through the
        # core's guarded flashConfirmed (C11 behind CG8, with a P1TK token),
        # called only by FirmwareInstaller.swift's FirmwareFlashCore; the
        # firmware files write no file and find no drive themselves. No Swift
        # direct-copy path is kept, not even behind -CoreTransport NO.
        stray=$(grep -rlE '\.flashConfirmed\(' --include='*.swift' apps/apple |
          grep -vE '/Core/Generated/|/DSPi Console/FirmwareInstaller\.swift$|/DSPi ConsoleTests/' || true)
        if [ -n "$stray" ]; then echo "swift: the core's firmware write is called outside FirmwareInstaller.swift:" >&2; echo "$stray" >&2; exit 1; fi
        fw=("apps/apple/DSPi Console/FirmwareInstaller.swift" "apps/apple/DSPi Console/BootloaderLocator.swift"
            "apps/apple/DSPi Console/FirmwareUpdateView.swift" "apps/apple/DSPi Console/FirmwareInstallUI.swift"
            "apps/apple/DSPi Console/Onboarding/GettingStartedView.swift")
        stray=$(grep -nE 'FileHandle\((forWritingTo|forWritingAtPath|forUpdating)|createFile\(atPath|\.write\((to|toFile|contentsOf):|OutputStream\((url|toFileAtPath):|fopen\(|copyItem\(|moveItem\(|getmntinfo|IOServiceMatching|INFO_UF2' "${fw[@]}" |
          grep -vE '^[^:]+:[0-9]+:[[:space:]]*//' || true)
        if [ -n "$stray" ]; then echo "swift: a firmware file writes or finds a drive itself (the core's job):" >&2; echo "$stray" >&2; exit 1; fi
        echo "swift: firmware goes to a drive only through the core's flashConfirmed, in FirmwareInstaller.swift"
        # -- WP SWS7 -- the effect views name their live settings
        # (DSPViewModel.LiveFloat, the core's registry paths), never an
        # opcode: the vendor codes are the opt-out's, in Commands.swift.
        stray=$(cd "apps/apple/DSPi Console" && grep -nE '\bREQ_[A-Z0-9_]+|\bliveRequest\b' \
          CrossfeedView.swift LoudnessView.swift VolumeLevellerView.swift PsychoacousticBassView.swift SubharmonicSynthView.swift || true)
        if [ -n "$stray" ]; then echo "swift: an effect view names an opcode (use DSPViewModel.LiveFloat):" >&2; echo "$stray" >&2; exit 1; fi
        echo "swift: the five effect views name no opcode"
      fi ;;
    licences) python3 ci/licence-inventory.py --gate --fetch-console ;;
    deny)
      podman image exists "$DENY" || ci/container/build-images.sh
      DSPI_IMAGE=$DENY $RUN "cargo deny --workspace --all-features --locked check licenses bans sources"
      # -- WP SECDENY -- RustSec advisories: refresh the database in the container
      # (CARGO_HOME/advisory-dbs, the shared cargo-home), and with no network use
      # the one fetched last; then check offline against it
      DSPI_IMAGE=$DENY $RUN 'if cargo deny fetch db; then echo "deny: advisory database refreshed"
        elif ls -d "$CARGO_HOME"/advisory-dbs/*/ >/dev/null 2>&1; then echo "deny: no network, using the cached advisory database" >&2
        else echo "deny: no advisory database: run once with network" >&2; exit 1; fi
        cargo deny --workspace --all-features --frozen check advisories' ;;
    firmware-digests)
      # -- WP SECFWV --
      python3 tools/test_firmware_manifest.py
      python3 tools/firmware_manifest.py --check ;;
    drift)
      # -- WP MR2 --
      $RUN "ci/drift.sh check" ;;
    conformance-ts)
      # -- WP HB8 --
      crates/dspi-hub/tests/conformance/run-clients.sh ts ;;
    web)
      # -- WP WB5 --
      apps/web/tools/ci.sh ;;
    esp-rig)
      # -- WP LK3 --
      tools/esp-rig/ci.sh ;;
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
        # -- WP SECCORE -- the graph above is the iOS one; its compile is not: ring
        # (rustls, the Link TLS) needs the Apple SDK for its C
        echo "ios-check: cargo check for aarch64-apple-ios skipped: ring needs the Apple SDK; compiled by ci/macvm.sh ios"' ;;
    *) echo "unknown step $s" >&2; exit 2 ;;
  esac
  ok "$s"
done
