#!/bin/bash
# minisign, from its official release, run in a container (WP P0E, PRO1 E5).
#
#   ci/minisign.sh [--mount HOST_PATH:CONTAINER_PATH[:ro]]... -- MINISIGN_ARGS...
#
# The binary is minisign 0.12's official Linux build, from
# https://github.com/jedisct1/minisign/releases/tag/0.12. It is downloaded once
# into ~/.cache/dspi-build/tools/ and must match the SHA-256 pinned below before
# every run. The pin was set on 2026-11-12 after the tarball's own signature
# (minisign-0.12-linux.tar.gz.minisig) verified against the public key in
# minisign's README at tag 0.12,
#   RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3,
# with Fedora 44's separately built minisign-0.12-3.fc44. To move to a new
# version, repeat that check and change MINISIGN_VERSION and MINISIGN_SHA256.
#
# It runs in the build image (COREMERGE in ci/container/images.conf) as the host
# user, with no network and no USB, and with SELinux labelling off for this
# container instead of relabelling what is mounted: a signing key keeps its own
# label. Each --mount is passed as a podman volume; minisign sees only those.
# Interactive when standard input is a terminal, so minisign can ask for a key's
# password. Runs on the host (podman, curl, sha256sum, tar).
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
# shellcheck source=container/images.conf
. "$here/ci/container/images.conf"

MINISIGN_VERSION=0.12
MINISIGN_SHA256=9a599b48ba6eb7b1e80f12f36b94ceca7c00b7a5173c95c3efc88d9822957e73
URL=https://github.com/jedisct1/minisign/releases/download/$MINISIGN_VERSION/minisign-$MINISIGN_VERSION-linux.tar.gz

B=${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}
tools=$B/tools/minisign-$MINISIGN_VERSION
tarball=$tools/minisign-$MINISIGN_VERSION-linux.tar.gz
bin=$tools/minisign-linux/x86_64/minisign

mounts=()
while [ $# -gt 0 ]; do
  case $1 in
    --mount) mounts+=(-v "${2:?--mount needs HOST:CONTAINER}"); shift 2 ;;
    --) shift; break ;;
    *) echo "ci/minisign.sh: options end with --, then minisign's own" >&2; exit 2 ;;
  esac
done

case $(uname -m) in
  x86_64) ;;
  *) echo "ci/minisign.sh: the pinned binary is x86_64; this host is $(uname -m)" >&2; exit 2 ;;
esac

if [ ! -f "$tarball" ]; then
  mkdir -p "$tools"
  curl -sSfL --max-time 120 -o "$tarball.part" "$URL"
  mv "$tarball.part" "$tarball"
fi
if ! echo "$MINISIGN_SHA256  $tarball" | sha256sum -c --status; then
  echo "ci/minisign.sh: $tarball is not the pinned minisign $MINISIGN_VERSION; nothing was run." >&2
  echo "  Delete it to download it again; if it still differs, stop and find out why." >&2
  exit 3
fi
# Extracted from the checked tarball every time: what runs is what was checked.
rm -rf "$tools/minisign-linux"
tar -xzf "$tarball" -C "$tools" --warning=no-unknown-keyword minisign-linux/x86_64/minisign
chmod 755 "$bin"

podman image exists "$COREMERGE" || {
  echo "ci/minisign.sh: no image $COREMERGE. Build it: ci/container/build-images.sh" >&2
  exit 2
}
tty=(-i)
[ -t 0 ] && tty=(-it)
mkdir -p "$B/work/empty"
exec podman run --rm "${tty[@]}" --network=none --security-opt label=disable \
  --userns=keep-id --user "$(id -u):$(id -g)" \
  -v "$B/work/empty":/sys/bus/usb/devices:ro \
  -v "$bin":/usr/local/bin/minisign:ro \
  "${mounts[@]}" \
  "$COREMERGE" minisign "$@"
