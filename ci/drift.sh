#!/bin/bash
# The drift step: regenerate the generated code and fail on any difference from
# what is committed. Generated code is never hand-edited; change its source, then
# `ci/run.sh 'ci/drift.sh write'`. Runs inside the build container, called by
# ci/check.sh --step drift.
#
#   ci/drift.sh check   regenerate into /work/drift, compare, exit 1 on a difference
#   ci/drift.sh write   the same, then replace ci/generated/ with the new output
#
# What it covers:
#   ci/generated/dspi_core.h   cbindgen's C header, as crates/dspi-capi's build script
#                              writes it (the header itself is gitignored)
#   ci/generated/kotlin/       UniFFI's Kotlin, one run per namespace
#   ci/generated/swift/        UniFFI's Swift, one run per namespace
#   apps/android core-api, ffi the Android contract (CoreTypes, DspiCoreApi, FfiDspiCore,
#                              UnimplementedDspiCore, DspiConstants), read from crates/
#                              by apps/android/tools/gen_kotlin_contract.py --check
#
#   apps/apple/.../Core/Generated/Sources/<ns>/<ns>.swift
#                              the Swift app's copy of ci/generated/swift (WP SW2),
#                              one package module per namespace
#
# ci/generated/ is the committed copy of output that the builds regenerate for
# themselves.
set -euo pipefail
mode=${1:?usage: drift.sh check|write}
case "$mode" in check|write) ;; *) echo "unknown mode $mode" >&2; exit 2 ;; esac

committed=ci/generated
out=/work/drift
rm -rf "$out"
mkdir -p "$out"

# The build script has `rerun-if-changed` on the crate's own files only: remove its
# record, so that the header really is regenerated and a stale one cannot pass.
rm -rf "$CARGO_TARGET_DIR"/debug/build/dspi-capi-*
cargo build --locked -p dspi-capi
cp crates/dspi-capi/include/dspi_core.h "$out/dspi_core.h"

cargo build --locked -p dspi-ffi --features cli
lib_dir=$CARGO_TARGET_DIR/debug
lib=$lib_dir/libdspi_ffi.so
mapfile -t namespaces < <(nm -D --defined-only "$lib" |
  sed -n 's/.* ffi_\(.*\)_uniffi_contract_version$/\1/p' | sort)
[ ${#namespaces[@]} -gt 0 ] || { echo "no UniFFI namespace in $lib" >&2; exit 1; }
for lang in kotlin swift; do
  for ns in "${namespaces[@]}"; do
    config=crates/${ns//_/-}/uniffi.toml
    [ -f "$config" ] || { echo "namespace $ns has no $config" >&2; exit 1; }
    "$lib_dir/uniffi-bindgen" generate --library "$lib" --crate "$ns" \
      --config "$config" --language "$lang" --no-format --out-dir "$out/$lang"
  done
done

status=0
if [ "$mode" = write ]; then
  rm -rf "$committed"
  cp -r "$out" "$committed"
  echo "drift: wrote $committed"
elif ! diff -ru "$committed" "$out" >"$out.diff"; then
  head -n 40 "$out.diff"
  echo "drift: $committed differs from what the sources generate (full diff in the" >&2
  echo "drift: container at $out.diff). Generated code is never hand-edited: if the" >&2
  echo "drift: source changed on purpose, run ci/run.sh 'ci/drift.sh write' and commit." >&2
  status=1
fi

# -- WP SW2 -- The Swift app's copy: the same files, and a module for each
# namespace (a new namespace also needs its target in the package's Package.swift).
app="apps/apple/DSPi Console/Core/Generated/Sources"
if [ "$mode" = write ]; then
  rm -rf "$app"
  for ns in "${namespaces[@]}"; do
    mkdir -p "$app/$ns"
    cp "$out/swift/$ns.swift" "$app/$ns/"
  done
  echo "drift: wrote $app"
else
  have=$(cd "$app" && ls | sort | tr '\n' ' ')
  if [ "$have" != "${namespaces[*]} " ]; then
    echo "drift: $app has modules [ $have] for namespaces [ ${namespaces[*]} ]" >&2
    status=1
  fi
  for ns in "${namespaces[@]}"; do
    if ! diff -q "$out/swift/$ns.swift" "$app/$ns/$ns.swift"; then
      echo "drift: $app/$ns/$ns.swift differs from what the sources generate" >&2
      status=1
    fi
  done
fi

if ! (cd apps/android && python3 tools/gen_kotlin_contract.py --check); then
  echo "drift: the Android contract is stale: apps/android/tools/gen_kotlin_contract.py" >&2
  status=1
fi
[ "$status" = 0 ] && echo "drift: no difference"
exit "$status"
