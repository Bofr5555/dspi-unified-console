#!/bin/bash
# Run a command in a build container, for the worktree this script is in.
#
#   ci/run.sh 'cargo test -p dspi-proto'
#   DSPI_PROFILE=cross ci/run.sh 'cargo xwin build -p dspi-core --target x86_64-pc-windows-msvc'
#
# Profiles:
#   default  COREMERGE in ci/container/images.conf (ci/container/Containerfile):
#            Linux, Windows GNU cross, Qt 5/6, rustfmt, clippy, ALSA, iOS std
#   cross    CROSS in ci/container/images.conf (ci/container/Containerfile.cross):
#            cargo-xwin (MSVC; the owner accepted Microsoft's CRT/SDK licence),
#            Android NDK r27 + cargo-ndk, wasm32
#
# ci/container/build-images.sh builds the images from their Containerfiles.
#
# Never uses tmpfs (/tmp, /dev/shm, a session scratchpad): everything lives
# under ~/.cache/dspi-build (override with DSPI_BUILD_CACHE):
#   target/<branch>[-cross]/  CARGO_TARGET_DIR per branch (/target)
#   cargo-home/               CARGO_HOME (/cache/cargo): shared registry and git caches
#   xwin-cache/               the MSVC CRT/SDK (cross profile)
#   work/<branch>/            scratch (/work; HOME and TMPDIR live there)
#
# Runs as the host user, not as the images' USER root: under --userns=keep-id
# root is uid 524288 on the host, and its files need `podman unshare` to remove.
# Caches written before this (WP MR3) are fixed once, with nothing building, by
#   podman unshare find ~/.cache/dspi-build -xdev \( -uid 1 -o -gid 1 \) -exec chown -h 0:0 {} +
#
# -- WP CIPAR -- DSPI_TARGET_KEY and DSPI_WORK_KEY name the target/ and work/
# directories in place of the branch (ci/full.sh gives each lane its own, so
# lanes do not wait on one cargo lock). Scripts that find /target's files on
# the host use the same key.
# SRC overrides the source tree (default: this worktree). DSPI_JOBS caps
# cargo's parallel jobs per container (default 4; several WPs build at once). No USB devices are
# visible in the container.
set -euo pipefail
here=${SRC:-$(cd "$(dirname "$0")/.." && pwd)}
# shellcheck source=container/images.conf
. "$(dirname "$0")/container/images.conf"
branch=$(git -C "$here" rev-parse --abbrev-ref HEAD 2>/dev/null || echo detached)
# -- WP SHOWFIX -- a detached worktree is named by its short commit, not "HEAD"
if [ "$branch" = HEAD ]; then branch=$(git -C "$here" rev-parse --short HEAD); fi
branch=${branch//\//-}
# -- WP CIPAR --
tkey=${DSPI_TARGET_KEY:-$branch}
branch=${DSPI_WORK_KEY:-$branch}
B=${DSPI_BUILD_CACHE:-$HOME/.cache/dspi-build}
profile=${DSPI_PROFILE:-default}
case "$profile" in
  default) image=${DSPI_IMAGE:-$COREMERGE} ;;
  cross) image=${DSPI_IMAGE:-$CROSS} ;;
  *) echo "unknown DSPI_PROFILE $profile" >&2; exit 2 ;;
esac
podman image exists "$image" || {
  echo "ci/run.sh: no image $image. Build it: ci/container/build-images.sh" >&2
  exit 2
}
mkdir -p "$B/cargo-home" "$B/work/$branch/home" \
         "$B/work/$branch/tmp" "$B/work/empty" "$B/xwin-cache"
common=(--rm -i --userns=keep-id --user "$(id -u):$(id -g)"
  -v "$here":/src:z -v "$B/work/$branch":/work:z
  -v "$B/work/empty":/sys/bus/usb/devices:ro,z
  -v "$B/cargo-home":/cache/cargo:z -e CARGO_HOME=/cache/cargo
  -e HOME=/work/home -e TMPDIR=/work/tmp -e CARGO_BUILD_JOBS="${DSPI_JOBS:-4}" -w /src)
# -- WP CIPAR -- cmake --build's jobs, when the caller caps them (ci/full.sh)
[ -z "${CMAKE_BUILD_PARALLEL_LEVEL:-}" ] || common+=(-e CMAKE_BUILD_PARALLEL_LEVEL="$CMAKE_BUILD_PARALLEL_LEVEL")
case "$profile" in
  default)
    mkdir -p "$B/target/$tkey"
    exec podman run "${common[@]}" -v "$B/target/$tkey":/target:z \
      -e CARGO_TARGET_DIR=/target \
      "$image" bash -c "$*" ;;
  cross)
    mkdir -p "$B/target/$tkey-cross"
    exec podman run "${common[@]}" -v "$B/target/$tkey-cross":/target:z \
      -v "$B/xwin-cache":/xwin:z \
      -e CARGO_TARGET_DIR=/target -e RUSTUP_HOME=/opt/rustup \
      -e XWIN_CACHE_DIR=/xwin -e ANDROID_NDK_HOME=/opt/android-sdk/ndk/27.2.12479018 \
      -e ANDROID_HOME=/opt/android-sdk \
      -e PATH=/opt/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
      "$image" bash -c "$*" ;;
esac
