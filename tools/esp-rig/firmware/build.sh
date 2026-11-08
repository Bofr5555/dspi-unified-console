#!/bin/bash
# WP LK3: build upstream's ESP32 bridge firmware for the emulator rig.
#
#   tools/esp-rig/firmware/build.sh [OUT]
#
# Takes DSPi-Link at the conformance pin (DSPI_LINK_COMMIT, from
# crates/dspi-hub/tests/conformance/run-clients.sh) out of a local clone
# (DSPI_LINK_REPO, default ~/.cache/dspi-build/work/consoles/DSPi-Link) with
# git archive, never touching the clone; replaces components/net with the
# rig's Ethernet one (firmware/net, the one deviation: QEMU has no Wi-Fi);
# builds it for the ESP32 in Espressif's ESP-IDF image at idf_version.txt,
# pinned by digest; and merges a 4 MB flash image, OUT/flash.bin (default
# ~/.cache/dspi-build/work/esp-rig/fw).
#
# RIG_IP sets the bridge's fixed address (default 10.77.0.2; /24, gateway .1).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../.." && pwd)
. "$here/../images.conf"
out=${1:-$HOME/.cache/dspi-build/work/esp-rig/fw}
link_repo=${DSPI_LINK_REPO:-$HOME/.cache/dspi-build/work/consoles/DSPi-Link}
commit=$(sed -n 's/^DSPI_LINK_COMMIT=//p' "$repo/crates/dspi-hub/tests/conformance/run-clients.sh")
[ -n "$commit" ] || { echo "no DSPI_LINK_COMMIT in run-clients.sh" >&2; exit 2; }
git -C "$link_repo" cat-file -e "$commit^{commit}" || {
  echo "the clone at $link_repo lacks $commit" >&2; exit 2; }

idf=$(git -C "$link_repo" show "$commit:bridge/esp32/idf_version.txt" | tr -d '[:space:]')
[ "$idf" = "$IDF_VERSION" ] || {
  echo "upstream wants ESP-IDF $idf, images.conf pins $IDF_VERSION: update IDF_IMAGE" >&2; exit 2; }
podman image exists "$IDF_IMAGE" || podman pull "$IDF_IMAGE"

src=$out/src
rm -rf "$src"
mkdir -p "$src" "$out/build"
git -C "$link_repo" archive "$commit" | tar -x -C "$src"
fw=$src/bridge/esp32
cp "$here/net/net_qemu.c" "$here/net/CMakeLists.txt" "$fw/components/net/"
rm "$fw/components/net/net.c"
cp "$here/sdkconfig.defaults.rig" "$fw/"
echo "$commit" > "$out/DSPI_LINK_COMMIT"

# The firmware embeds upstream's web pages (components/web_assets): built as
# crates/dspi-hub/tools/build-link-web.sh builds them, npm ci from upstream's
# lockfile (integrity hashes, no install scripts) in the pinned Node image.
. "$repo/ci/container/images.conf"
podman image exists "$NODE" || podman pull -q "$NODE" > /dev/null
mkdir -p "$out/npm" "$out/home"
podman run --rm --init --userns=keep-id --user "$(id -u):$(id -g)" \
  -v "$src":/w:z -v "$out/npm":/npm:z -v "$out/home":/home/n:z \
  -e HOME=/home/n -e npm_config_cache=/npm -e npm_config_update_notifier=false \
  -w /w/js "$NODE" bash -euo pipefail -c '
    npm ci --no-audit --no-fund --ignore-scripts
    npm run build'

# IDF applies sdkconfig.defaults.esp32 after each listed file by itself.
podman run --rm -i --cpus 4 --userns=keep-id --user "$(id -u):$(id -g)" \
  -v "$src":/repo:z -v "$out/build":/build:z -e HOME=/build/home \
  -e RIG_IP="${RIG_IP:-10.77.0.2}" -w /repo/bridge/esp32 "$IDF_IMAGE" bash -c '
    set -e; mkdir -p /build/home
    . $IDF_PATH/export.sh > /dev/null
    idf.py -B /build/esp32 -DIDF_TARGET=esp32 -DSDKCONFIG=/build/esp32/sdkconfig \
      -DSDKCONFIG_DEFAULTS="sdkconfig.defaults;sdkconfig.defaults.rig" build
    cd /build/esp32
    python -m esptool --chip esp32 merge-bin --fill-flash-size 4MB -o /build/flash.bin @flash_args'
cp "$out/build/flash.bin" "$out/flash.bin"
echo "flash image: $out/flash.bin (DSPi-Link $commit, ESP-IDF $IDF_VERSION)"
