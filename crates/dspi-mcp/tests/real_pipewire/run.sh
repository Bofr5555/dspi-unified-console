#!/bin/bash
# WP FEAT3: dspi_get_host_audio's Linux reader against a real PipeWire.
# Builds dspi-mcp's test binary in the build container (ci/run.sh), then runs
# its ignored real-PipeWire test in a PipeWire container (inner.sh): null
# sinks only, no sound device, no network, no USB.
#   crates/dspi-mcp/tests/real_pipewire/run.sh
# PW_IMAGE picks the image (default localhost/dspi-feat3-pw:1, built from
# the Containerfile here when missing: Fedora 44 like the build container,
# whose glibc the test binary needs). Scratch and the
# captured pw-dump go to ~/.cache/dspi-build/work/wp-FEAT3/real-pw.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../../.." && pwd)
B=${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}
branch=$(git -C "$repo" rev-parse --abbrev-ref HEAD); branch=${branch//\//-}
out="$B/work/wp-FEAT3/real-pw"; mkdir -p "$out" "$B/work/empty"
image=${PW_IMAGE:-localhost/dspi-feat3-pw:1}
podman image exists "$image" || podman build -t "$image" -f "$here/Containerfile" "$here"
bin=$("$repo/ci/run.sh" 'cargo test -p dspi-mcp --features host-audio --lib --no-run --message-format=json 2>/dev/null' \
  | python3 -I -c 'import json,sys
for l in sys.stdin:
    try: m=json.loads(l)
    except ValueError: continue
    if m.get("reason")=="compiler-artifact" and m.get("executable") and m["target"]["name"]=="dspi_mcp": print(m["executable"])' | tail -1)
[ -n "$bin" ] || { echo "run.sh: no test binary" >&2; exit 2; }
host_bin="$B/target/$branch/${bin#/target/}"
podman run --rm --network none --userns=keep-id --user "$(id -u):$(id -g)" \
  -v "$B/work/empty":/sys/bus/usb/devices:ro,z \
  -v "$host_bin":/t/test-binary:ro,z -v "$here/inner.sh":/t/inner.sh:ro,z -v "$out":/out:z \
  "$image" bash /t/inner.sh
