#!/bin/bash
# -- WP POSTSWAP -- The Qt desktop app's files for a release folder
# (docs/RELEASING.md), as ci/release-terminal.sh makes the Terminal's.
#
#   ci/release-desktop.sh [--test] [VERSION [OUTDIR]]
#
# Runs apps/desktop/packaging/build-release.sh without a key into its own
# folder, $DSPI_BUILD_CACHE/work/release-VERSION: the Windows zip and setup,
# the AppImage, the linux-x86_64.tar.gz, the .deb, one .rpm per Fedora in
# FEDORAS, and uConsole-for-DSPi-VERSION-desktop-source.tar.gz
# (ci/release-source.sh --with-desktop), which the packages' notices name.
# Only the Windows files, the AppImage and the tar.gz carry the update feed;
# the .deb, the .rpm and the AUR package never do (packaging/README.md).
# Nothing is signed here: ci/release-sums.sh signs the whole release folder as
# its last step.
#
# VERSION, when given, must be the app's (qt/CMakeLists.txt): every client
# shares one version. With OUTDIR the files are then copied into it, beside
# the release's other files; a file already there is never replaced. Without
# OUTDIR they stay in the build folder, whose path is printed.
# --test: before copying, packaging/linux/test-packages.sh installs each Linux
# file in clean containers and starts it on the demo device; its logs stay in
# the build folder, never in OUTDIR. The Windows setup's test under Wine is
# ci/check.sh --step qt-windows.
#
# Runs on the host from a clean commit (podman, the network for cargo's crates
# and the distributions' packages). ci/matrix.sh's row qt-packages runs
# `ci/release-desktop.sh --test`.
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
B=${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}
test=0
if [ "${1:-}" = --test ]; then test=1; shift; fi
app=$(sed -n 's/^project(uconsole-for-dspi VERSION \([0-9.]*\).*/\1/p' "$here/apps/desktop/qt/CMakeLists.txt")
[ -n "$app" ] || { echo "release-desktop.sh: no version in apps/desktop/qt/CMakeLists.txt" >&2; exit 1; }
version=${1:-$app}
out=${2:-}
if [ "$version" != "$app" ]; then
  echo "release-desktop.sh: the app's version is $app, not $version (apps/desktop/qt/CMakeLists.txt)" >&2
  exit 2
fi
built=$B/work/release-$version
# build-release.sh empties its own folder first, so a copy into it is refused
if [ -n "$out" ]; then
  mkdir -p "$out"
  [ "$(cd "$out" && pwd)" != "$built" ] || { echo "release-desktop.sh: OUTDIR is build-release.sh's own folder; name another" >&2; exit 2; }
fi

DSPI_MINISIGN_KEY= "$here/apps/desktop/packaging/build-release.sh"
if [ "$test" = 1 ]; then
  # test-packages.sh writes its logs next to the files: test them in a copy
  t=$B/work/release-desktop-test
  rm -rf "$t"; mkdir -p "$t"
  cp "$built"/* "$t/"
  "$here/apps/desktop/packaging/linux/test-packages.sh" "$t"
fi
[ -n "$out" ] || { echo "release-desktop.sh: the files are in $built"; exit 0; }
for f in "$built"/*; do
  n=$(basename "$f")
  [ ! -e "$out/$n" ] || { echo "release-desktop.sh: $out/$n is there already; nothing replaced" >&2; exit 1; }
done
cp "$built"/* "$out/"
for f in "$built"/*; do echo "$out/$(basename "$f")"; done
