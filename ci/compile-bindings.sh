#!/bin/bash
# Generate the UniFFI bindings from the built library and compile them, one
# language per run. Runs inside the build container (ci/check.sh --step kotlin,
# --step swift), which has the compilers under /opt (ci/container/Containerfile).
#
# The namespaces come from the library itself, so a new one is compiled without
# editing this script. It must have crates/<namespace>/uniffi.toml.
#
#   kotlin  every namespace's .kt in one kotlinc run, as one Gradle module would.
#           Two namespaces in one package clash here.
#   swift   each namespace as its own module against its FFI header, then linked
#           against the library, so a symbol the binding calls must exist.
set -euo pipefail
lang=${1:?usage: compile-bindings.sh kotlin|swift}
case "$lang" in kotlin|swift) ;; *) echo "unknown language $lang" >&2; exit 2 ;; esac

cargo build --locked -p dspi-ffi --features cli
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

case "$lang" in
  kotlin)
    mapfile -t sources < <(find "$out/src" -name '*.kt' | sort)
    [ ${#sources[@]} -eq ${#namespaces[@]} ] || {
      echo "${#namespaces[@]} namespaces, ${#sources[@]} Kotlin files" >&2; exit 1; }
    JAVA_HOME=/opt/jdk JAVA_OPTS=-Xmx3g /opt/kotlinc/bin/kotlinc -nowarn \
      -cp /opt/kotlin-libs/jna.jar:/opt/kotlin-libs/coroutines.jar \
      -d "$out/build/bindings.jar" "${sources[@]}"
    echo "compiled ${#sources[@]} Kotlin files: ${sources[*]##*/}"
    ;;
  swift)
    # Swift 5 mode, the language mode of the Apple app's Xcode project.
    for ns in "${namespaces[@]}"; do
      /opt/swift/bin/swiftc -swift-version 5 -parse-as-library -module-name "$ns" \
        -emit-library -use-ld=lld -Xlinker --no-undefined \
        -I "$out/src" -Xcc "-fmodule-map-file=$out/src/${ns}FFI.modulemap" \
        -L "$lib_dir" -ldspi_ffi \
        -emit-module -emit-module-path "$out/build/$ns.swiftmodule" \
        -o "$out/build/lib$ns.so" "$out/src/$ns.swift"
    done
    echo "compiled and linked ${#namespaces[@]} Swift modules: ${namespaces[*]}"
    ;;
esac
