#!/bin/bash
# Build the DSPi core (crates/dspi-ffi) as an XCFramework for the Swift app (WP SW1).
# macOS only: it needs Xcode and the Apple Rust targets, so it runs in the macOS VM.
#
#   apps/apple/build-xcframework.sh              build the XCFramework
#   apps/apple/build-xcframework.sh --link-test  build it, then link a test against it
#                                                and run it on macOS and in the iOS
#                                                Simulator, and link it for an iOS device
#
# Slices (static libraries, release profile):
#   ios-arm64                      aarch64-apple-ios                      `usb` off
#   ios-arm64_x86_64-simulator     aarch64-apple-ios-sim + x86_64-apple-ios  `usb` off
#   macos-arm64_x86_64             aarch64-apple-darwin + x86_64-apple-darwin  `usb` on
# iOS has no nusb backend, so dspi-ffi builds there with --no-default-features
# (WP CG3); the API is the same, its USB methods report no devices.
#
# Output in $OUT (default apps/apple/build/xcframework, gitignored):
#   DSPiCore.xcframework   the libraries, the namespaces' FFI headers and one
#                          module.modulemap with a clang module per namespace
#   swift/                 UniFFI's Swift, one file per namespace (as ci/drift.sh
#                          generates it; the app's copy is SW2's)
#   native-static-libs.txt what each slice must also link (from rustc)
#   SIZES.txt              library sizes: each static library, and the stripped
#                          cdylib as the size the core adds to a linked app
#
# Needs: Xcode, rustup with the five targets above (`rustup target add ...`).
# The simulator run needs the GUI session: from ssh, run it through
#   sudo launchctl asuser "$(id -u)" sudo -u "$USER" apps/apple/build-xcframework.sh --link-test
# CARGO_TARGET_DIR defaults to target/ in the repository; cargo runs at most
# CARGO_BUILD_JOBS (default 4) jobs.
set -euo pipefail

link_test=0
case "${1:-}" in
  "") ;;
  --link-test) link_test=1 ;;
  *) echo "usage: $0 [--link-test]" >&2; exit 2 ;;
esac

[ "$(uname -s)" = Darwin ] || { echo "build-xcframework: macOS only (the macOS VM)" >&2; exit 2; }
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$root/target}
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}
out=${OUT:-$root/apps/apple/build/xcframework}
# The Xcode project's lowest deployment targets.
export IPHONEOS_DEPLOYMENT_TARGET=17.0
export MACOSX_DEPLOYMENT_TARGET=14.6
name=DSPiCore

ios_targets=(aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios)
mac_targets=(aarch64-apple-darwin x86_64-apple-darwin)
installed=$(rustup target list --installed)
for t in "${ios_targets[@]}" "${mac_targets[@]}"; do
  grep -qx "$t" <<<"$installed" || { echo "build-xcframework: missing Rust target $t (rustup target add $t)" >&2; exit 1; }
done

mkdir -p "$out"
: >"$out/native-static-libs.txt"

# One slice's library. Only the staticlib and cdylib crate types: with the
# crate's `lib` (rlib) in the build, cargo keeps LLVM bitcode in every object
# for a later LTO, which made the archive four times its size and unreadable to
# Xcode's nm. The cdylib is the bindings' input and the measure of linked size.
# rustc prints the native libraries a static library needs only when it compiles
# it, so the list is kept next to the library for a later run where cargo finds
# it fresh.
build() {
  local target=$1; shift
  local log=$CARGO_TARGET_DIR/$target/release/dspi_ffi.build.log
  local libs=$CARGO_TARGET_DIR/$target/release/dspi_ffi.native-static-libs
  mkdir -p "$(dirname "$log")"
  echo "== $target $*"
  cargo rustc --release --locked -p dspi-ffi --lib --crate-type staticlib,cdylib --target "$target" "$@" \
    -- --print native-static-libs 2>&1 | tee "$log"
  if grep -q 'native-static-libs:' "$log"; then
    sed -n 's/.*native-static-libs: //p' "$log" | head -n 1 >"$libs"
  fi
  [ -s "$libs" ] || { echo "build-xcframework: no native-static-libs for $target; touch crates/dspi-ffi/src/lib.rs and run again" >&2; exit 1; }
  echo "$target: $(cat "$libs")" >>"$out/native-static-libs.txt"
}
for t in "${ios_targets[@]}"; do build "$t" --no-default-features; done
for t in "${mac_targets[@]}"; do build "$t"; done

lib() { echo "$CARGO_TARGET_DIR/$1/release/libdspi_ffi.a"; }
slices=$out/slices
rm -rf "$slices"
mkdir -p "$slices/ios" "$slices/ios-simulator" "$slices/macos"
cp "$(lib aarch64-apple-ios)" "$slices/ios/libdspi_ffi.a"
lipo -create "$(lib aarch64-apple-ios-sim)" "$(lib x86_64-apple-ios)" -output "$slices/ios-simulator/libdspi_ffi.a"
lipo -create "$(lib aarch64-apple-darwin)" "$(lib x86_64-apple-darwin)" -output "$slices/macos/libdspi_ffi.a"

# The bindings come from the host's dylib (library mode), as in ci/drift.sh: the
# namespaces from its symbols, one bindgen run each. They match ci/generated/swift.
echo "== bindings"
cargo build --release --locked -p dspi-ffi --features cli --bin uniffi-bindgen
bindgen=$CARGO_TARGET_DIR/release/uniffi-bindgen
dylib=$CARGO_TARGET_DIR/$(uname -m | sed 's/arm64/aarch64/')-apple-darwin/release/libdspi_ffi.dylib
namespaces=()
while IFS= read -r ns; do namespaces+=("$ns"); done < <(nm -gU "$dylib" |
  sed -n 's/.* _ffi_\(.*\)_uniffi_contract_version$/\1/p' | sort)
[ ${#namespaces[@]} -gt 0 ] || { echo "build-xcframework: no UniFFI namespace in $dylib" >&2; exit 1; }
rm -rf "$out/swift" "$out/headers"
mkdir -p "$out/headers"
for ns in "${namespaces[@]}"; do
  config=crates/${ns//_/-}/uniffi.toml
  [ -f "$config" ] || { echo "build-xcframework: namespace $ns has no $config" >&2; exit 1; }
  "$bindgen" generate --library "$dylib" --crate "$ns" --config "$config" \
    --language swift --no-format --out-dir "$out/swift"
  cp "$out/swift/${ns}FFI.h" "$out/headers/"
  { cat "$out/swift/${ns}FFI.modulemap"; echo; } >>"$out/headers/module.modulemap"
done
echo "namespaces: ${namespaces[*]}"

echo "== $name.xcframework"
rm -rf "$out/$name.xcframework"
xcodebuild -create-xcframework \
  -library "$slices/ios/libdspi_ffi.a" -headers "$out/headers" \
  -library "$slices/ios-simulator/libdspi_ffi.a" -headers "$out/headers" \
  -library "$slices/macos/libdspi_ffi.a" -headers "$out/headers" \
  -output "$out/$name.xcframework"

size() { stat -f %z "$1"; }
{
  echo "dspi-ffi, release profile ($(date -u +%Y-%m-%dT%H:%MZ), $(rustc -V))"
  printf '  %-22s %10s  %10s\n' target "static .a" "linked (cdylib, stripped)"
  for t in "${ios_targets[@]}" "${mac_targets[@]}"; do
    printf '  %-22s %10s  %10s bytes\n' "$t" "$(size "$(lib "$t")")" "$(size "${CARGO_TARGET_DIR}/$t/release/libdspi_ffi.dylib")"
  done
  for s in ios ios-simulator macos; do
    printf '  %-22s %10s bytes  (%s)\n' "slice $s" "$(size "$slices/$s/libdspi_ffi.a")" "$(lipo -archs "$slices/$s/libdspi_ffi.a")"
  done
  printf '  %-22s %10s bytes\n' "$name.xcframework" "$(du -sk "$out/$name.xcframework" | awk '{print $1 * 1024}')"
} >"$out/SIZES.txt"
cat "$out/SIZES.txt"
echo "built $out/$name.xcframework"

[ "$link_test" = 1 ] || exit 0

# The link test: a Swift package with the XCFramework as its binary target, one
# Swift module per namespace (as ci/compile-bindings.sh), and a test that calls
# into both. Generated here, so it always matches the bindings.
echo "== link test"
pkg=$out/linktest
rm -rf "$pkg"
mkdir -p "$pkg/Tests/LinkTests"
# Frameworks and libraries per platform, from rustc's lists.
settings() {
  local platform=$1 libs=$2 prev="" res=""
  for f in $libs; do
    if [ "$prev" = -framework ]; then
      res+="        .linkedFramework(\"$f\", .when(platforms: [.$platform])),"$'\n'
    elif [[ $f == -l* ]]; then
      res+="        .linkedLibrary(\"${f#-l}\", .when(platforms: [.$platform])),"$'\n'
    fi
    prev=$f
  done
  printf '%s' "$res"
}
mac_libs=$(sed -n 's/^aarch64-apple-darwin: //p' "$out/native-static-libs.txt")
ios_libs=$(sed -n 's/^aarch64-apple-ios: //p' "$out/native-static-libs.txt")
targets=""
deps=""
for ns in "${namespaces[@]}"; do
  mkdir -p "$pkg/Sources/$ns"
  cp "$out/swift/$ns.swift" "$pkg/Sources/$ns/"
  targets+="    .target(name: \"$ns\", dependencies: [\"$name\"], linkerSettings: [
$(settings macOS "$mac_libs")
$(settings iOS "$ios_libs")
    ]),"$'\n'
  deps+="\"$ns\", "
done
cat >"$pkg/Package.swift" <<EOF
// swift-tools-version:5.9
// Generated by apps/apple/build-xcframework.sh --link-test (WP SW1).
import PackageDescription

let package = Package(
  name: "LinkTest",
  platforms: [.macOS("$MACOSX_DEPLOYMENT_TARGET"), .iOS("$IPHONEOS_DEPLOYMENT_TARGET")],
  targets: [
    .binaryTarget(name: "$name", path: "../$name.xcframework"),
$targets    .testTarget(name: "LinkTests", dependencies: [${deps%, }]),
  ],
  swiftLanguageVersions: [.v5]
)
EOF
cat >"$pkg/Tests/LinkTests/LinkTests.swift" <<'EOF'
// Generated by apps/apple/build-xcframework.sh --link-test (WP SW1).
import XCTest
import dspi_core
import dspi_ffi

final class LinkTests: XCTestCase {
  // Calls into both namespaces: the library links, its checksums match the
  // bindings, and the core answers.
  func testSW1CoreInfo() {
    initLogging()
    let info = coreInfo()
    print("SW1 link test: core \(info.ffiVersion), wire \(info.wireFormatVersion) (min \(info.minWireFormatVersion)), bulk \(info.bulkParamsSize), curve \(info.magnitudePoints)")
    XCTAssertFalse(info.ffiVersion.isEmpty)
    XCTAssertGreaterThanOrEqual(info.wireFormatVersion, info.minWireFormatVersion)
    XCTAssertGreaterThan(info.magnitudePoints, 0)
  }
}
EOF

dd=$out/linktest-dd
cd "$pkg"
run() { echo "-- xcodebuild $*"; xcodebuild -quiet -scheme LinkTest-Package -derivedDataPath "$dd" "$@"; }
# A test run counts only with exactly one test, passed.
test_on() {
  local label=$1 dest=$2 bundle=$out/linktest-$1.xcresult summary
  rm -rf "$bundle"
  run test -destination "$dest" -resultBundlePath "$bundle"
  summary=$(xcrun xcresulttool get test-results summary --path "$bundle")
  grep -Eq '"totalTestCount" *: *1,?$' <<<"$summary" && grep -Eq '"passedTests" *: *1,?$' <<<"$summary" || {
    echo "$summary" >&2; echo "build-xcframework: $label: the link test did not run and pass" >&2; exit 1; }
  echo "$label: 1 test, passed"
}
test_on macos "platform=macOS,arch=$(uname -m | sed 's/aarch64/arm64/')"
# Any available iPhone simulator; xcodebuild boots it.
sim=$(xcrun simctl list devices available | sed -n 's/^ *\(iPhone[^(]*\) (\([0-9A-F-]\{36\}\)).*/\2 \1/p' | head -n 1)
[ -n "$sim" ] || { echo "build-xcframework: no iPhone simulator available" >&2; exit 1; }
echo "simulator: $sim"
test_on ios-simulator "id=${sim%% *}"
run build-for-testing -destination generic/platform=iOS CODE_SIGNING_ALLOWED=NO

{
  echo "linked test bundles (dead-stripped, debug Swift):"
  for b in "$dd"/Build/Products/*/LinkTests.xctest; do
    bin=$b/LinkTests
    [ -f "$bin" ] || bin=$b/Contents/MacOS/LinkTests
    printf '  %-40s %10s bytes  (%s)\n' "$(basename "$(dirname "$b")")" "$(size "$bin")" "$(lipo -archs "$bin")"
  done
} | tee -a "$out/SIZES.txt"
echo "link test passed: macOS, iOS Simulator ($sim), iOS device link"
