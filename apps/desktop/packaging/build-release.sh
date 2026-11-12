#!/usr/bin/env bash
# -- WP QP20 -- Builds every release file of the Qt desktop app, each in its
# own image, from a clean commit, and gathers them in one flat folder:
#
#   apps/desktop/packaging/build-release.sh [--key MINISIGN_SECRET_KEY] [output dir]
#
# Output dir: default $DSPI_BUILD_CACHE/work/release-<version> (never tmpfs),
# emptied first since it is this script's own; a dir you name must be empty
# or not exist yet, so nothing of yours is ever deleted.
# In it (the names packaging/README.md lists; the app's update checker relies
# on the first four):
#   DSPi-uConsole-<v>-windows-x86_64.zip           windows/build-windows.sh
#   DSPi-uConsole-<v>-windows-x86_64-setup.exe     windows/build-windows.sh
#   DSPi-uConsole-<v>-x86_64.AppImage              linux/build-ubuntu.sh + make-appimage.sh
#   DSPi-uConsole-<v>-linux-x86_64.tar.gz          linux/build-ubuntu.sh + make-appimage.sh
#   dspi-uconsole_<v>_amd64.deb                    linux/build-debian.sh (Debian 13)
#   dspi-uconsole-<v>-1.fc<N>.x86_64.rpm           linux/build-fedora.sh, one per Fedora in FEDORAS
#   dspi-uconsole-<v>-desktop-source.tar.gz        ci/release-source.sh --with-desktop
# Then, with a key (--key or DSPI_MINISIGN_KEY): SHA256SUMS and
# SHA256SUMS.minisig by ci/release-sums.sh. Without one nothing is signed and
# no SHA256SUMS is written: sign the folder later with ci/release-sums.sh.
# The release's library sources come from release-sources.sh (README.md).
#
# Runs on the host: podman builds the images from their Containerfiles (once;
# tags dspi-uconsole-*) and runs each build as this user, with the network
# (cargo's crates) and without USB. Scratch under $DSPI_BUILD_CACHE/work/release-build.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../.." && pwd)
B=${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}
FEDORAS=${FEDORAS:-43 44}
key=${DSPI_MINISIGN_KEY:-}
if [ "${1:-}" = --key ]; then key=${2:?--key needs a file}; shift 2; fi
version=$(sed -n 's/^project(DSPiConsole VERSION \([0-9.]*\).*/\1/p' "$here/../qt/CMakeLists.txt")
[ -n "$version" ] || { echo "build-release: no version in qt/CMakeLists.txt" >&2; exit 1; }
own=$B/work/release-$version
out=${1:-$own}
if [ "$out" != "$own" ] && [ -d "$out" ] && [ -n "$(ls -A "$out")" ]; then
    echo "build-release: $out is not empty; name an empty or new folder" >&2; exit 2
fi
[ -z "$(git -C "$repo" status --porcelain)" ] || { echo "build-release: the work tree has changes; a release is built from a commit" >&2; exit 1; }
[ -z "$key" ] || [ -f "$key" ] || { echo "build-release: no key file at $key" >&2; exit 2; }

scratch=$B/work/release-build
mkdir -p "$scratch/empty" "$scratch/dist" "$B/cargo-home" "$out"
dist=$scratch/dist
rm -f "$dist"/*

image() {  # <tag> <Containerfile> <context> [build args...]
    podman image exists "$1" || podman build -t "$1" -f "$2" "${@:4}" "$3"
}
run() {  # <image> <name> <command>: the tree at /src, scratch at /work, files to /dist
    mkdir -p "$scratch/$2"
    podman run --rm --userns=keep-id --user "$(id -u):$(id -g)" --security-opt label=disable \
        -v "$repo":/src -v "$scratch/$2":/work -v "$dist":/dist -v "$B/cargo-home":/cache/cargo \
        -v "$scratch/empty":/sys/bus/usb/devices:ro \
        -e HOME=/work -e CARGO_HOME=/cache/cargo -e CARGO_TARGET_DIR=/work/target -e CARGO_BUILD_JOBS="${DSPI_JOBS:-4}" \
        -e BUILD_DIR=/work/build -e PATH=/opt/cargo/bin:/opt/qt/bin:/usr/local/bin:/usr/bin:/bin -w /src \
        "$1" bash -c "$3"
}

L=$here/linux
image dspi-uconsole-ubuntu "$L/Containerfile.ubuntu" "$L"
run dspi-uconsole-ubuntu ubuntu 'apps/desktop/packaging/linux/build-ubuntu.sh && apps/desktop/packaging/linux/make-appimage.sh /work/build/AppDir /dist'
image dspi-uconsole-debian "$L/Containerfile.debian" "$L"
run dspi-uconsole-debian debian 'apps/desktop/packaging/linux/build-debian.sh /dist'
for fc in $FEDORAS; do
    image "dspi-uconsole-fedora:$fc" "$L/Containerfile.fedora" "$L" --build-arg "FEDORA=$fc"
    run "dspi-uconsole-fedora:$fc" "fedora$fc" 'apps/desktop/packaging/linux/build-fedora.sh /dist'
done
image dspi-uconsole-windows "$here/windows/Containerfile" "$here/windows"
run dspi-uconsole-windows windows 'apps/desktop/packaging/windows/build-windows.sh /work/dist && cp /work/dist/DSPi-uConsole-*-windows-x86_64.zip /work/dist/DSPi-uConsole-*-windows-x86_64-setup.exe /dist/'
"$repo/ci/release-source.sh" --with-desktop "$version" "$dist" >/dev/null

# The release folder: exactly the files above, nothing else (release-sums.sh
# sums every file in it)
want=("DSPi-uConsole-$version-windows-x86_64.zip" "DSPi-uConsole-$version-windows-x86_64-setup.exe"
      "DSPi-uConsole-$version-x86_64.AppImage" "DSPi-uConsole-$version-linux-x86_64.tar.gz"
      "dspi-uconsole_${version}_amd64.deb" "dspi-uconsole-$version-desktop-source.tar.gz")
for fc in $FEDORAS; do want+=("dspi-uconsole-$version-1.fc$fc.x86_64.rpm"); done
[ "$out" != "$own" ] || rm -rf "$own"
mkdir -p "$out"
for f in "${want[@]}"; do
    [ -f "$dist/$f" ] || { echo "build-release: $f was not made" >&2; exit 1; }
    cp "$dist/$f" "$out/"
done
if [ -n "$key" ]; then
    "$repo/ci/release-sums.sh" --key "$key" --comment "uConsole for DSPi $version" "$out"
else
    echo "build-release: unsigned: no SHA256SUMS written. Sign with: ci/release-sums.sh --key <key> $out"
fi
ls -la "$out"
