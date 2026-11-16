#!/bin/bash
# Generate the UniFFI bindings from the built library and compile them, one
# language per run. Runs inside the build container (ci/check.sh --step kotlin,
# --step swift), which has the compilers under /opt (ci/container/Containerfile).
#
# The namespaces come from the library itself, so a new one is compiled without
# editing this script. It must have crates/<namespace>/uniffi.toml.
#
#   kotlin  every namespace's .kt in one kotlinc run, as one Gradle module would.
#           Two namespaces in one package clash here. With them the Android
#           contract and its adapter (apps/android core-api, FfiDspiCore.kt): both
#           are pure JVM, and a binding that changed under them fails here.
#   swift   each namespace as its own module against its FFI header, then linked
#           against the library, so a symbol the binding calls must exist.
set -euo pipefail
lang=${1:?usage: compile-bindings.sh kotlin|swift}
case "$lang" in kotlin|swift) ;; *) echo "unknown language $lang" >&2; exit 2 ;; esac

# -- WP SW-M3 -- with `hub`, as the Mac app's library: every namespace
cargo build --locked -p dspi-ffi --features cli,hub
lib_dir=$CARGO_TARGET_DIR/debug
lib=$lib_dir/libdspi_ffi.so

mapfile -t namespaces < <(nm -D --defined-only "$lib" |
  sed -n 's/.* ffi_\(.*\)_uniffi_contract_version$/\1/p' | sort)
[ ${#namespaces[@]} -gt 0 ] || { echo "no UniFFI namespace in $lib" >&2; exit 1; }
echo "namespaces: ${namespaces[*]}"

out=/work/bindings/$lang
rm -rf "$out"
mkdir -p "$out/src" "$out/build"
for ns in "${namespaces[@]}"; do
  config=crates/${ns//_/-}/uniffi.toml
  [ -f "$config" ] || { echo "namespace $ns has no $config" >&2; exit 1; }
  "$lib_dir/uniffi-bindgen" generate --library "$lib" --crate "$ns" \
    --config "$config" --language "$lang" --no-format --out-dir "$out/src"
done
# -- WP SW-M3 -- one namespace's Swift using another's types imports it
if [ "$lang" = swift ]; then python3 ci/uniffi_swift_imports.py "$out"/src/*.swift; fi

case "$lang" in
  kotlin)
    mapfile -t sources < <(find "$out/src" -name '*.kt' | sort)
    [ ${#sources[@]} -eq ${#namespaces[@]} ] || {
      echo "${#namespaces[@]} namespaces, ${#sources[@]} Kotlin files" >&2; exit 1; }
    # -- WP AN1 --
    mapfile -t -O "${#sources[@]}" sources < <(find apps/android/core-api/src/main/kotlin apps/android/ffi/src/main/kotlin -name '*.kt' | sort)
    JAVA_HOME=/opt/jdk JAVA_OPTS=-Xmx3g /opt/kotlinc/bin/kotlinc -nowarn \
      -cp /opt/kotlin-libs/jna.jar:/opt/kotlin-libs/coroutines.jar \
      -d "$out/build/bindings.jar" "${sources[@]}"
    echo "compiled ${#sources[@]} Kotlin files: ${sources[*]##*/}"
    ;;
  swift)
    # Swift 5 mode, the language mode of the Apple app's Xcode project.
    # -- WP SW-M3 -- a namespace may use another's types (dspi_hub takes dspi_core's
    # DspiCore): every module sees the ones built before it, in name order, and
    # links against them. The library's directory comes first: the module
    # dspi_ffi is a libdspi_ffi.so of its own in $out/build.
    built=()
    for ns in "${namespaces[@]}"; do
      deps=()
      for b in ${built[@]+"${built[@]}"}; do
        deps+=(-Xcc "-fmodule-map-file=$out/src/${b}FFI.modulemap" "-l$b")
      done
      /opt/swift/bin/swiftc -swift-version 5 -parse-as-library -module-name "$ns" \
        -emit-library -use-ld=lld -Xlinker --no-undefined \
        -I "$out/src" -Xcc "-fmodule-map-file=$out/src/${ns}FFI.modulemap" \
        -L "$lib_dir" -ldspi_ffi \
        -I "$out/build" -L "$out/build" ${deps[@]+"${deps[@]}"} \
        -emit-module -emit-module-path "$out/build/$ns.swiftmodule" \
        -o "$out/build/lib$ns.so" "$out/src/$ns.swift"
      built+=("$ns")
    done
    echo "compiled and linked ${#namespaces[@]} Swift modules: ${namespaces[*]}"
    # -- WP SW25 -- The generated constants (the committed copy, which the drift
    # step holds to the sources), as the app's package module, and a check that
    # their types and values reach a user of the module.
    consts="apps/apple/DSPi Console/Core/Generated/Sources/dspi_constants/DspiConstants.swift"
    /opt/swift/bin/swiftc -swift-version 5 -parse-as-library -module-name dspi_constants \
      -emit-library -use-ld=lld -emit-module -emit-module-path "$out/build/dspi_constants.swiftmodule" \
      -o "$out/build/libdspi_constants.so" "$consts"
    /opt/swift/bin/swiftc -swift-version 5 -use-ld=lld -I "$out/build" -L "$out/build" -ldspi_constants \
      -o "$out/build/constants_check_sw25" ci/swift/constants_check_sw25.swift
    LD_LIBRARY_PATH="$out/build" "$out/build/constants_check_sw25"
    ;;
esac
